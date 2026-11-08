import { test, before, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { createServer as createH2 } from "node:http2";
import { createServer } from "node:http";
import { createHash, createPublicKey, generateKeyPairSync, verify } from "node:crypto";
import { createKey, push, MAX_PAYLOAD_BYTES, BOUND_SECONDS, BAD_KEYS_PER_HOUR, BIND } from "../lib/relay.js";
import { host } from "../lib/apns.js";

const BUNDLE = "com.example.loft";
const DEVICE = "a1".repeat(32);
const OTHER_DEVICE = "b2".repeat(32);
const { privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
const P8 = privateKey.export({ type: "pkcs8", format: "pem" });

// The store: Upstash's REST API, keeping every command it gets and the path of each request (one round trip each).
// Expiries run on a clock the tests move with `later`.
const store = new Map();
const sets = new Map();
const commands = [];
const requests = [];
const deadlines = new Map();
let clock = 0;
const expire = (key, seconds) => deadlines.set(key, clock + Number(seconds));
/** Moves the store's clock on by `seconds`, dropping what expired. */
const later = (seconds) => {
  clock += seconds;
  for (const [key, at] of deadlines) if (at <= clock) (store.delete(key), sets.delete(key), deadlines.delete(key));
};
const run = ([cmd, key, ...args]) => {
  // The relay's one script (BIND) runs as the commands it makes, which are kept instead.
  if (cmd === "EVAL") return bind(key, args);
  commands.push(JSON.stringify([cmd, key, ...args]));
  if (cmd === "SET") {
    if (args.includes("NX") && store.has(key)) return null;
    store.set(key, args[0]);
    const ex = args.indexOf("EX");
    return (ex > 0 ? expire(key, args[ex + 1]) : deadlines.delete(key), "OK");
  }
  if (cmd === "GET") return store.get(key) ?? null;
  if (cmd === "EXISTS") return store.has(key) ? 1 : 0;
  if (cmd === "INCR") return (store.set(key, String(Number(store.get(key) ?? 0) + 1)), Number(store.get(key)));
  if (cmd === "EXPIRE") return store.has(key) || sets.has(key) ? ((args[1] === "NX" && deadlines.has(key)) || expire(key, args[0]), 1) : 0;
  if (cmd === "SADD") return (sets.set(key, (sets.get(key) ?? new Set()).add(args[0])), 1);
  return null;
};
/** BIND, as Redis runs it. */
const bind = (script, [n, ...rest]) => {
  assert.equal(script, BIND, "the only script the relay runs");
  const [[owner, seconds, device], [holds, list]] = [rest.slice(n), rest.slice(0, n)];
  const holder = run(["GET", holds]);
  if (holder === null) run(["SET", holds, owner, "EX", seconds]);
  else if (holder === owner) run(["EXPIRE", holds, seconds]);
  else return 0;
  if (list) (run(["SADD", list, device]), run(["EXPIRE", list, seconds]));
  return 1;
};
const kvServer = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", () => {
    if (req.headers.authorization !== "Bearer kv-secret") return res.writeHead(401).end();
    requests.push(req.url);
    const parsed = JSON.parse(body);
    const out = req.url === "/" ? { result: run(parsed) } : parsed.map((c) => ({ result: run(c) }));
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
  // Each test starts with every phone unbound, so a new key can push to DEVICE.
  for (const key of store.keys()) if (key.startsWith("device-owner:")) store.delete(key);
  production.seen.length = 0;
  development.seen.length = 0;
  production.answer({ status: 200 });
});

// What the operator's script does for each user.
const newKey = () => createKey(env, "a test install");
const payload = { aps: { alert: { title: "Web Dev needs your approval", body: "Bash" }, sound: "default", badge: 3 }, type: "permission" };
const send = (token, body, raw, now, ip) =>
  push(
    new Request("http://relay/api/push", {
      method: "POST",
      headers: { ...(token && { authorization: `Bearer ${token}` }), ...(ip && { "x-forwarded-for": ip }) },
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
  assert.ok(!commands.slice(commandsBefore).some((c) => c.includes("relay-key:")), "a malformed key costs no key lookup");
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
  // Only hashed keys, counters keyed by those hashes, and which key holds the device, by the token's hash.
  assert.ok(
    commands.every((c) => /^\["(SET|GET)","relay-key:[0-9a-f]{64}"|^\["SET","relay-key-used:[0-9a-f]{64}","[\d-]+T[\d:.]+Z"|^\["(INCR|EXPIRE|GET)","limit:\w+:[0-9a-f]{64}:\d+"|^\["SET","device-owner:[0-9a-f]{64}","key:[0-9a-f]{64}","EX",\d+\]$|^\["(GET|EXPIRE)","device-owner:[0-9a-f]{64}"/.test(c)),
    commands.join("\n"),
  );
});

const sha256 = (s) => createHash("sha256").update(s).digest("hex");

test("the first push to a device binds it to its key: another key's push gets 403 and never reaches Apple", async () => {
  const [mine, theirs] = [await newKey(), await newKey()];
  assert.equal((await send(mine)).status, 200);
  assert.equal(store.get(`device-owner:${sha256(DEVICE)}`), `key:${sha256(mine)}`, "an operator key without an account holds it itself");
  const refused = await send(theirs);
  assert.equal(refused.status, 403);
  assert.deepEqual(await refused.json(), { error: "this device belongs to another account: only that account's relay keys can push to it" });
  assert.equal(production.seen.length, 1, "the refused push wasn't forwarded");
  for (const spelling of [DEVICE.toUpperCase(), "A1a1".repeat(16)]) {
    assert.equal((await send(theirs, { device_token: spelling })).status, 403, `${spelling} is the same device`);
  }
  assert.equal(production.seen.length, 1, "no spelling of it was forwarded");
  assert.equal((await send(mine, { device_token: DEVICE.toUpperCase() })).status, 200, "the key holding it pushes in any case");
  assert.equal(production.seen.at(-1).path, `/3/device/${DEVICE}`, "Apple gets the token lower-case");
  assert.equal((await send(mine)).status, 200, "the key holding it still pushes");
  assert.equal((await send(theirs, { device_token: OTHER_DEVICE })).status, 200, "another phone is free");
  assert.ok(![...store.keys(), ...store.values()].some((v) => v.toLowerCase().includes(DEVICE)), "only the token's hash is stored");
});

test("a device's binding expires 30 days after the last push to it, and each push keeps it", async () => {
  const [mine, theirs] = [await newKey(), await newKey()];
  assert.equal(BOUND_SECONDS, 30 * 86400);
  assert.equal((await send(mine)).status, 200);
  later(BOUND_SECONDS - 60);
  assert.equal((await send(mine)).status, 200, "a push a day before it expires starts it over");
  later(BOUND_SECONDS - 60);
  assert.equal((await send(theirs)).status, 403, "still held 59 days after the first push");
  later(60);
  assert.equal((await send(theirs)).status, 200, "30 idle days later the device is free, and the next push binds it");
  assert.equal((await send(mine)).status, 403);
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
  assert.equal((await send(b, { device_token: OTHER_DEVICE }, undefined, T0 + 30_000)).status, 200, "another key");
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
  assert.equal((await send(await newKey(), { device_token: OTHER_DEVICE }, undefined, day + 51 * 60_000)).status, 200, "another key");
  assert.equal((await send(token, {}, undefined, day + 24 * HOUR)).status, 200, "the next day");
});

test("past 60 refused keys in an hour, one address gets 429 even with a good key; other addresses and the next hour don't", async () => {
  const [max, seconds] = BAD_KEYS_PER_HOUR;
  assert.deepEqual([max, seconds], [60, 3600]);
  const [ip, other] = ["203.0.113.7", "198.51.100.4"];
  const good = await newKey();
  const unknown = `lrk_${"c".repeat(43)}`;
  for (let i = 0; i < max; i++) {
    const res = await send(i % 2 ? unknown : "garbage", {}, undefined, T0 + i * 1000, `${ip}, 10.0.0.1`);
    assert.equal(res.status, 401, `refusal ${i + 1}`);
  }
  for (const key of [unknown, good]) {
    const before = commands.length;
    const over = await send(key, {}, undefined, T0 + 30 * 60_000, ip);
    assert.equal(over.status, 429, key === good ? "a valid key from that address" : "another bad key");
    assert.deepEqual(await over.json(), { error: "too many refused relay keys: at most 60 per hour from one address" });
    assert.equal(over.headers.get("retry-after"), "1800");
    // Its key is read with the address's refusals, in one round trip, but the key is never used or counted.
    assert.ok(!commands.slice(before).some((c) => /relay-key-used:|limit:pushes|device-owner:/.test(c)), "refused before its key is used");
  }
  assert.equal(production.seen.length, 0, "nothing reached Apple");
  assert.equal((await send(unknown, {}, undefined, T0 + 30 * 60_000, other)).status, 401, "another address still gets 401");
  assert.equal((await send(good, {}, undefined, T0 + 30 * 60_000, other)).status, 200, "and a good key from it pushes");
  assert.equal((await send(good, {}, undefined, T0 + HOUR, ip)).status, 200, "the next hour starts over");
  const written = [...commands, ...store.keys(), ...store.values()].join("\n");
  assert.ok(!written.includes(ip) && !written.includes(other), "the store never sees an address");
  assert.ok(store.has(`limit:badKeys:${sha256(ip)}:${Math.floor(T0 / 1000 / seconds)}`), "only its hash");
});

test("a forwarded push costs five store round trips: refusals and key, two limits, last used, binding", async () => {
  const [token, other] = [await newKey(), await newKey()];
  requests.length = 0;
  assert.equal((await send(token)).status, 200);
  assert.deepEqual(requests, ["/pipeline", "/pipeline", "/pipeline", "/", "/"]);
  requests.length = 0;
  assert.equal((await send(other)).status, 403, "a refused push too");
  assert.equal(requests.length, 5);
  requests.length = 0;
  assert.equal((await send("garbage")).status, 401);
  assert.deepEqual(requests, ["/pipeline", "/pipeline"], "a bad key: the refusals read, then one more counted");
});

test("a valid key's pushes never count against its address", async () => {
  const token = await newKey();
  const ip = "192.0.2.55";
  for (let i = 0; i < 20; i++) assert.equal((await send(token, {}, undefined, T0 + 2 * HOUR + i * 1000, ip)).status, 200);
  assert.ok(![...store.keys()].some((k) => k.startsWith(`limit:badKeys:${sha256(ip)}:`)), "no refusal was counted");
  assert.equal((await send("garbage", {}, undefined, T0 + 2 * HOUR, ip)).status, 401, "a bad key from it is still only a 401");
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
  const allowed = /^(appleid\.apple\.com|api(\.sandbox)?\.push\.apple\.com|developer\.apple\.com|openapi\.vercel\.sh|portal\.get-loft\.com|(loft-push-relay|<project>)\.vercel\.app|[\w.-]*example(\.(com|org))?|relay|127\.0\.0\.1)$/;
  for (const f of files) {
    const text = await readFile(new URL(`../${f}`, import.meta.url), "utf8");
    for (const [, host] of text.matchAll(/https?:\/\/([^/\s"'`)<>:${]+)/g)) assert.match(host, allowed, `${f}: ${host}`);
    for (const [id] of text.matchAll(/\b(com|is|io)\.[a-z0-9-]+\.[a-z0-9.-]+/g)) assert.match(id, /^com\.example\./, `${f}: ${id}`);
  }
});
