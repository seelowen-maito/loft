import { test, before, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { createServer as createH2 } from "node:http2";
import { createServer } from "node:http";
import { createHash, createPublicKey, generateKeyPairSync, verify } from "node:crypto";
import { issueToken, push, MAX_PAYLOAD_BYTES } from "../lib/relay.js";

const BUNDLE = "com.example.loft";
const DEVICE = "a1".repeat(32);
const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
const P8 = privateKey.export({ type: "pkcs8", format: "pem" });

// The store: Upstash's REST API, keeping every command it gets.
const store = new Map();
const commands = [];
const kvServer = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", () => {
    const [cmd, key, value] = JSON.parse(body);
    commands.push(body);
    if (req.headers.authorization !== "Bearer kv-secret") return res.writeHead(401).end();
    const incr = () => (store.set(key, String(Number(store.get(key) ?? 0) + 1)), Number(store.get(key)));
    const result =
      cmd === "SET" ? (store.set(key, value), "OK")
      : cmd === "EXISTS" ? (store.has(key) ? 1 : 0)
      : cmd === "INCR" ? incr()
      : cmd === "EXPIRE" ? 1
      : null;
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ result }));
  });
});

// Apple, one fake per environment: records each request and answers what the test set.
const fakeApple = () => {
  const seen = [];
  let answer = { status: 200 };
  const server = createH2((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      seen.push({ path: req.headers[":path"], headers: req.headers, body });
      res.writeHead(answer.status, { "apns-id": "apns-1" });
      res.end(answer.reason ? JSON.stringify({ reason: answer.reason }) : "");
    });
  });
  return { server, seen, answer: (a) => (answer = a) };
};
const production = fakeApple();
const development = fakeApple();

const listen = (s) => new Promise((r) => s.listen(0, "127.0.0.1", () => r(`http://127.0.0.1:${s.address().port}`)));
let env;
before(async () => {
  env = {
    KV_REST_API_URL: await listen(kvServer),
    KV_REST_API_TOKEN: "kv-secret",
    APNS_HOST_PRODUCTION: await listen(production.server),
    APNS_HOST_DEVELOPMENT: await listen(development.server),
    APNS_KEY_P8: P8,
    APNS_KEY_ID: "ABC123DEFG",
    APNS_TEAM_ID: "TEAM123456",
    LOFT_BUNDLE_ID: BUNDLE,
  };
});
after(() => [kvServer, production.server, development.server].forEach((s) => s.close()));
beforeEach(() => {
  production.seen.length = 0;
  development.seen.length = 0;
  production.answer({ status: 200 });
});

// Each install asks from its own IP, so the per-IP limit only bites where a test means it to.
let ips = 0;
const askToken = (ip = `10.0.${Math.floor(++ips / 250)}.${ips % 250}`, now) =>
  issueToken(new Request("http://relay/api/token", { method: "POST", headers: { "x-forwarded-for": `${ip}, 76.76.21.21` } }), env, now);
const newToken = async () => (await (await askToken()).json()).token;
const payload = { aps: { alert: { title: "Web Dev needs your approval", body: "Bash" }, sound: "default", badge: 3 }, type: "permission" };
const send = (token, body, raw, now) =>
  push(
    new Request("http://relay/api/push", {
      method: "POST",
      headers: token ? { authorization: `Bearer ${token}` } : {},
      body: raw ?? JSON.stringify({ device_token: DEVICE, environment: "production", bundle_id: BUNDLE, payload, ...body }),
    }),
    env,
    now,
  );

test("a relay token is a secret: only its hash is stored", async () => {
  const res = await askToken();
  assert.equal(res.status, 201);
  const { token } = await res.json();
  assert.match(token, /^lrt_[A-Za-z0-9_-]{43}$/);
  assert.ok(store.has(`relay-token:${createHash("sha256").update(token).digest("hex")}`));
  assert.ok(![...store.keys(), ...store.values()].some((v) => v.includes(token)), "the token itself isn't stored");
  assert.notEqual(await newToken(), token, "each install gets its own");
});

test("a push is forwarded to Apple's host for its environment, with the Loft topic and the payload unchanged", async () => {
  const token = await newToken();
  const res = await send(token, { priority: 5, collapse_id: "permission:abc" });
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { apple_status: 200, reason: null, apns_id: "apns-1" });
  assert.equal(development.seen.length, 0);
  const [r] = production.seen;
  assert.equal(r.path, `/3/device/${DEVICE}`);
  assert.deepEqual(JSON.parse(r.body), payload);
  assert.equal(r.headers["apns-topic"], BUNDLE);
  assert.equal(r.headers["apns-push-type"], "alert");
  assert.equal(r.headers["apns-priority"], "5");
  assert.equal(r.headers["apns-collapse-id"], "permission:abc");
  // An ES256 provider token signed with the operator's key, for its key id and team.
  const [h, c, s] = r.headers.authorization.replace(/^bearer /, "").split(".");
  assert.deepEqual(JSON.parse(Buffer.from(h, "base64url")), { alg: "ES256", kid: "ABC123DEFG" });
  assert.equal(JSON.parse(Buffer.from(c, "base64url")).iss, "TEAM123456");
  assert.ok(verify("sha256", Buffer.from(`${h}.${c}`), { key: createPublicKey(privateKey), dsaEncoding: "ieee-p1363" }, Buffer.from(s, "base64url")));

  await send(token, { environment: "development" });
  assert.equal(development.seen.length, 1, "a development build goes to the sandbox host");
  assert.equal(development.seen[0].headers["apns-priority"], "10", "time-sensitive by default");
  assert.equal(production.seen.length, 1);
});

test("a missing or unknown relay token is refused, and nothing reaches Apple", async () => {
  for (const token of [null, "lrt_not-issued", "Basic x"]) {
    const res = await send(token);
    assert.equal(res.status, 401, String(token));
    assert.deepEqual(await res.json(), { error: "a valid relay token is required" });
  }
  assert.equal(production.seen.length, 0);
});

test("over 4 KB, another bundle id and a malformed request are refused before Apple", async () => {
  const token = await newToken();
  const big = { aps: { alert: { title: "x", body: "y".repeat(MAX_PAYLOAD_BYTES) } } };
  assert.equal((await send(token, { payload: big })).status, 413);
  const justUnder = { aps: { alert: { title: "x", body: "" } }, pad: "" };
  justUnder.pad = "z".repeat(MAX_PAYLOAD_BYTES - Buffer.byteLength(JSON.stringify(justUnder)));
  assert.equal((await send(token, { payload: justUnder })).status, 200, "exactly 4096 bytes is allowed");
  assert.equal((await send(token, {}, "x".repeat(3 * MAX_PAYLOAD_BYTES))).status, 413);

  const other = await send(token, { bundle_id: "com.someone.else" });
  assert.equal(other.status, 403);
  assert.match((await other.json()).error, /only serves the Loft app's bundle id/);

  for (const bad of [
    { device_token: "../../x" },
    { device_token: "" },
    { environment: "staging" },
    { payload: "text" },
    { payload: [1] },
    { priority: 7 },
    { collapse_id: "c".repeat(65) },
  ]) {
    assert.equal((await send(token, bad)).status, 400, JSON.stringify(bad));
  }
  assert.equal((await send(token, {}, "{not json")).status, 400);
  assert.equal(production.seen.length, 1, "only the 4096-byte push was forwarded");
});

test("Apple's answer about a gone or bad device token comes back to the caller", async () => {
  const token = await newToken();
  for (const [status, reason] of [[410, "Unregistered"], [400, "BadDeviceToken"], [400, "DeviceTokenNotForTopic"]]) {
    production.answer({ status, reason });
    const res = await send(token);
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), { apple_status: status, reason, apns_id: "apns-1" });
  }
});

test("no payload or device token is written to the store or to the logs", async () => {
  const logged = [];
  const methods = ["log", "info", "warn", "error", "debug"];
  const original = Object.fromEntries(methods.map((m) => [m, console[m]]));
  methods.forEach((m) => (console[m] = (...a) => logged.push(a.map(String).join(" "))));
  try {
    commands.length = 0;
    const token = await newToken();
    await send(token);
    production.answer({ status: 410, reason: "Unregistered" });
    await send(token);
    await send(token, { bundle_id: "com.someone.else" });
  } finally {
    Object.assign(console, original);
  }
  const written = [...commands, ...store.keys(), ...store.values()].join("\n");
  for (const secret of [DEVICE, "Web Dev needs your approval", "permission"]) {
    assert.ok(!written.includes(secret), `the store saw ${secret}`);
    assert.ok(!logged.join("\n").includes(secret), `the logs have ${secret}`);
  }
  // Only hashed tokens and counters keyed by hashes: no IP either.
  assert.ok(
    commands.every((c) => /^\["(SET|EXISTS)","relay-token:[0-9a-f]{64}"|^\["(INCR|EXPIRE)","limit:\w+:[0-9a-f]{64}:\d+"/.test(c)),
    commands.join("\n"),
  );
  assert.ok(!written.includes("10.0."), "no IP is stored");
});

test("when Apple can't be reached the caller is told, without details", async () => {
  const token = await newToken();
  const res = await push(
    new Request("http://relay/api/push", {
      method: "POST",
      headers: { authorization: `Bearer ${token}` },
      body: JSON.stringify({ device_token: DEVICE, environment: "production", bundle_id: BUNDLE, payload }),
    }),
    { ...env, APNS_HOST_PRODUCTION: "http://127.0.0.1:1" },
  );
  assert.equal(res.status, 502);
  assert.deepEqual(await res.json(), { error: "couldn't reach Apple" });
});

const HOUR = 3600_000;
const T0 = Date.UTC(2026, 10, 4, 10, 0, 0);

test("the 6th token from one IP within an hour is refused; other IPs aren't, and the next hour starts over", async () => {
  for (let i = 0; i < 5; i++) assert.equal((await askToken("203.0.113.7", T0 + i * 60_000)).status, 201);
  const sixth = await askToken("203.0.113.7", T0 + 30 * 60_000);
  assert.equal(sixth.status, 429);
  assert.deepEqual(await sixth.json(), { error: "too many requests: at most 5 per hour" });
  assert.equal(sixth.headers.get("retry-after"), String(30 * 60));
  assert.equal((await askToken("203.0.113.8", T0 + 30 * 60_000)).status, 201, "another IP");
  assert.equal((await askToken("203.0.113.7", T0 + HOUR)).status, 201, "the next hour");
});

test("the 21st push in a minute from one token is refused; other tokens aren't, and the next minute starts over", async () => {
  const [a, b] = [await newToken(), await newToken()];
  for (let i = 0; i < 20; i++) assert.equal((await send(a, {}, undefined, T0 + i * 1000)).status, 200);
  const over = await send(a, {}, undefined, T0 + 30_000);
  assert.equal(over.status, 429);
  assert.deepEqual(await over.json(), { error: "too many requests: at most 20 per minute" });
  assert.equal(over.headers.get("retry-after"), "30");
  assert.equal(production.seen.length, 20, "the refused push never reached Apple");
  assert.equal((await send(b, {}, undefined, T0 + 30_000)).status, 200, "another token");
  assert.equal((await send(a, {}, undefined, T0 + 60_000)).status, 200, "the next minute");
});

test("the 1,001st push in a day from one token is refused until the next day", async () => {
  const token = await newToken();
  const day = Date.UTC(2026, 10, 5);
  // 20 a minute, so only the daily limit is reached.
  for (let i = 0; i < 1000; i++) {
    const res = await send(token, {}, undefined, day + Math.floor(i / 20) * 60_000);
    assert.equal(res.status, 200, `push ${i + 1}`);
  }
  const over = await send(token, {}, undefined, day + 51 * 60_000);
  assert.equal(over.status, 429);
  assert.deepEqual(await over.json(), { error: "too many requests: at most 1000 per day" });
  assert.equal((await send(await newToken(), {}, undefined, day + 51 * 60_000)).status, 200, "another token");
  assert.equal((await send(token, {}, undefined, day + 24 * HOUR)).status, 200, "the next day");
});
