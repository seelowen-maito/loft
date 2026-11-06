import { test, before, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { createServer as createH2 } from "node:http2";
import { createServer } from "node:http";
import { createHash, createPublicKey, generateKeyPairSync, verify } from "node:crypto";
import { createKey, push, MAX_PAYLOAD_BYTES } from "../lib/relay.js";
import { host } from "../lib/apns.js";

const BUNDLE = "com.example.loft";
const DEVICE = "a1".repeat(32);
const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
const P8 = privateKey.export({ type: "pkcs8", format: "pem" });

// The store: Upstash's REST API, keeping every command it gets.
const store = new Map();
const commands = [];
const run = ([cmd, key, value]) => {
  commands.push(JSON.stringify([cmd, key, value]));
  if (cmd === "SET") return (store.set(key, value), "OK");
  if (cmd === "EXISTS") return store.has(key) ? 1 : 0;
  if (cmd === "INCR") return (store.set(key, String(Number(store.get(key) ?? 0) + 1)), Number(store.get(key)));
  if (cmd === "EXPIRE") return 1;
  return null;
};
const kvServer = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", () => {
    if (req.headers.authorization !== "Bearer kv-secret") return res.writeHead(401).end();
    const parsed = JSON.parse(body);
    const out = req.url === "/pipeline" ? parsed.map((c) => ({ result: run(c) })) : { result: run(parsed) };
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(out));
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
      if (answer.hang) return;
      if (answer.reset) return req.stream.close(0x2);
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

// What the operator's script does for each user.
const newKey = () => createKey(env, "a test install");
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

test("a relay key is a secret: only its hash and label are stored", async () => {
  const key = await createKey(env, "Sam (sam@example.com)", Date.UTC(2026, 10, 4));
  assert.match(key, /^lrk_[A-Za-z0-9_-]{43}$/);
  const stored = store.get(`relay-key:${createHash("sha256").update(key).digest("hex")}`);
  assert.deepEqual(JSON.parse(stored), { label: "Sam (sam@example.com)", created: "2026-11-04T00:00:00.000Z" });
  assert.ok(![...store.keys(), ...store.values()].some((v) => v.includes(key)), "the key itself isn't stored");
  assert.notEqual(await newKey(), key, "each one is new");
});

test("the only endpoint that hands out keys is the portal's: behind Sign in with Apple, or a code a signed-in user approved", async () => {
  const { readdir } = await import("node:fs/promises");
  assert.deepEqual((await readdir(new URL("../api/", import.meta.url))).sort(), ["account-delete.js", "auth-callback.js", "auth-start.js", "connect.js", "keys.js", "pair-poll.js", "pair.js", "portal.js", "push.js", "revoke.js"]);
});

test("a push is forwarded to Apple's host for its environment, with the Loft topic and the payload unchanged", async () => {
  const token = await newKey();
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

test("a missing, malformed or unknown relay key is refused, and nothing reaches Apple", async () => {
  const commandsBefore = commands.length;
  for (const key of [null, "Basic x", "lrk_short", `lrt_${"a".repeat(43)}`]) {
    const res = await send(key);
    assert.equal(res.status, 401, String(key));
    assert.deepEqual(await res.json(), { error: "a valid relay key is required" });
  }
  assert.equal(commands.length, commandsBefore, "a malformed key costs no store lookup");
  const unknown = await send(`lrk_${"a".repeat(43)}`);
  assert.equal(unknown.status, 401, "well formed but never created");
  assert.equal(production.seen.length, 0);
});

test("over 4 KB, another bundle id and a malformed request are refused before Apple", async () => {
  const token = await newKey();
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
    { device_token: "a1".repeat(31) + "a" },
    { device_token: "a1".repeat(33) },
    { collapse_id: "line\nbreak" },
    { collapse_id: "café" },
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
  const token = await newKey();
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
    const token = await newKey();
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
  // Only hashed keys and counters keyed by those hashes.
  assert.ok(
    commands.every((c) => /^\["(SET|EXISTS)","relay-key:[0-9a-f]{64}"|^\["SET","relay-key-used:[0-9a-f]{64}","[\d-]+T[\d:.]+Z"|^\["(INCR|EXPIRE)","limit:\w+:[0-9a-f]{64}:\d+"/.test(c)),
    commands.join("\n"),
  );
});

test("when Apple can't be reached the caller is told, without details", async () => {
  const token = await newKey();
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

test("the 21st push in a minute from one key is refused; other keys aren't, and the next minute starts over", async () => {
  const [a, b] = [await newKey(), await newKey()];
  for (let i = 0; i < 20; i++) assert.equal((await send(a, {}, undefined, T0 + i * 1000)).status, 200);
  const over = await send(a, {}, undefined, T0 + 30_000);
  assert.equal(over.status, 429);
  assert.deepEqual(await over.json(), { error: "too many requests: at most 20 per minute" });
  assert.equal(over.headers.get("retry-after"), "30");
  assert.equal(production.seen.length, 20, "the refused push never reached Apple");
  assert.equal((await send(b, {}, undefined, T0 + 30_000)).status, 200, "another key");
  assert.equal((await send(a, {}, undefined, T0 + 60_000)).status, 200, "the next minute");
});

test("the 1,001st push in a day from one key is refused until the next day", async () => {
  const token = await newKey();
  const day = Date.UTC(2026, 10, 5);
  // 20 a minute, so only the daily limit is reached.
  for (let i = 0; i < 1000; i++) {
    const res = await send(token, {}, undefined, day + Math.floor(i / 20) * 60_000);
    assert.equal(res.status, 200, `push ${i + 1}`);
  }
  const over = await send(token, {}, undefined, day + 51 * 60_000);
  assert.equal(over.status, 429);
  assert.deepEqual(await over.json(), { error: "too many requests: at most 1000 per day" });
  assert.equal((await send(await newKey(), {}, undefined, day + 51 * 60_000)).status, 200, "another key");
  assert.equal((await send(token, {}, undefined, day + 24 * HOUR)).status, 200, "the next day");
});

test("a relay missing a setting refuses everything instead of guessing", async () => {
  for (const missing of ["LOFT_BUNDLE_ID", "APNS_KEY_P8", "KV_REST_API_URL"]) {
    const broken = { ...env, [missing]: "" };
    // Without LOFT_BUNDLE_ID a request that leaves bundle_id out would otherwise match.
    const res = await push(
      new Request("http://relay/api/push", {
        method: "POST",
        headers: { authorization: `Bearer lrk_${"a".repeat(43)}` },
        body: JSON.stringify({ device_token: DEVICE, environment: "production", payload }),
      }),
      broken,
    );
    assert.equal(res.status, 500, missing);
    assert.deepEqual(await res.json(), { error: "the relay isn't configured" });
  }
  assert.equal(production.seen.length, 0);
});

test("Apple's hosts can only be replaced outside production", () => {
  assert.equal(host(env, "production"), env.APNS_HOST_PRODUCTION);
  assert.equal(host({ ...env, VERCEL_ENV: "production" }, "production"), "https://api.push.apple.com");
  assert.equal(host({ ...env, VERCEL_ENV: "production" }, "development"), "https://api.sandbox.push.apple.com");
});

test("Apple not answering in time is a 504, and the next push still goes", async () => {
  const token = await newKey();
  production.answer({ hang: true });
  const started = Date.now();
  const res = await push(
    new Request("http://relay/api/push", {
      method: "POST",
      headers: { authorization: `Bearer ${token}` },
      body: JSON.stringify({ device_token: DEVICE, environment: "production", bundle_id: BUNDLE, payload }),
    }),
    env,
    Date.now(),
    200,
  );
  assert.equal(res.status, 504);
  assert.deepEqual(await res.json(), { error: "Apple didn't answer in time" });
  assert.ok(Date.now() - started < 2000);
  production.answer({ status: 200 });
  assert.equal((await send(token)).status, 200);
});

test("Apple closing the stream without an answer is a 502, never a 200", async () => {
  const token = await newKey();
  production.answer({ reset: true });
  const res = await send(token);
  assert.equal(res.status, 502);
  assert.deepEqual(await res.json(), { error: "couldn't reach Apple" });
});

test("a key that can't be used is the relay's 500, not Apple's outage", async () => {
  const token = await newKey();
  const res = await push(
    new Request("http://relay/api/push", {
      method: "POST",
      headers: { authorization: `Bearer ${token}` },
      body: JSON.stringify({ device_token: DEVICE, environment: "production", bundle_id: BUNDLE, payload }),
    }),
    { ...env, APNS_KEY_P8: "-----BEGIN PRIVATE KEY-----\nnope\n-----END PRIVATE KEY-----", APNS_KEY_ID: "OTHERKEY01" },
  );
  assert.equal(res.status, 500);
  assert.deepEqual(await res.json(), { error: "the relay's Apple key can't be used" });
});

test("an oversized request is refused from its length, before reading it", async () => {
  const token = await newKey();
  const res = await push(
    new Request("http://relay/api/push", { method: "POST", headers: { authorization: `Bearer ${token}`, "content-length": "999999" }, body: "{}" }),
    env,
  );
  assert.equal(res.status, 413);
});

test("relay/ holds no one's own hosts or ids: every value comes from the environment", async () => {
  const { readdir, readFile } = await import("node:fs/promises");
  const files = (await readdir(new URL("../", import.meta.url), { recursive: true })).filter((f) => /\.(js|json|md)$/.test(f) && !f.includes("node_modules") && !f.startsWith("test"));
  const allowed = /^(appleid\.apple\.com|api(\.sandbox)?\.push\.apple\.com|developer\.apple\.com|openapi\.vercel\.sh|(loft-push-relay|<project>)\.vercel\.app|[\w.-]*example(\.(com|org))?|relay|127\.0\.0\.1)$/;
  for (const f of files) {
    const text = await readFile(new URL(`../${f}`, import.meta.url), "utf8");
    for (const [, host] of text.matchAll(/https?:\/\/([^/\s"'`)<>:${]+)/g)) assert.match(host, allowed, `${f}: ${host}`);
    for (const [id] of text.matchAll(/\b(com|is|io)\.[a-z0-9-]+\.[a-z0-9.-]+/g)) assert.match(id, /^com\.example\./, `${f}: ${id}`);
  }
});
