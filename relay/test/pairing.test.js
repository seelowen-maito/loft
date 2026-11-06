import { test, before, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { generateKeyPairSync, createHash } from "node:crypto";
import { start, callback, home } from "../lib/portal.js";
import { pair, poll, connect, approve } from "../lib/pairing.js";
import { push } from "../lib/relay.js";

const SERVICES_ID = "com.example.loft.web";
const signIn = generateKeyPairSync("ec", { namedCurve: "P-256" });
const pem = (k) => k.privateKey.export({ type: "pkcs8", format: "pem" });

// The store, with expiry on a clock the tests move.
let clock = 0;
const store = new Map();
const sets = new Map();
const live = (k) => store.has(k) && !(store.get(k).until <= clock);
const run = ([cmd, key, value, opt, secs]) => {
  if (cmd === "SET") {
    if (opt === "NX" && live(key)) return null;
    store.set(key, { value, until: opt === "EX" ? clock + secs * 1000 : Infinity });
    return "OK";
  }
  if (cmd === "GET") return live(key) ? store.get(key).value : null;
  if (cmd === "GETDEL") return live(key) ? (({ value }) => (store.delete(key), value))(store.get(key)) : null;
  if (cmd === "EXISTS") return live(key) ? 1 : 0;
  if (cmd === "INCR") return (store.set(key, { value: String(Number(store.get(key)?.value ?? 0) + 1), until: Infinity }), Number(store.get(key).value));
  if (cmd === "DEL") return store.delete(key) ? 1 : 0;
  if (cmd === "SADD") return (sets.get(key) ?? sets.set(key, new Set()).get(key)).add(value) && sets.get(key).size;
  if (cmd === "SCARD") return sets.get(key)?.size ?? 0;
  if (cmd === "SMEMBERS") return [...(sets.get(key) ?? [])];
  if (cmd === "SREM") return sets.get(key)?.delete(value) ? 1 : 0;
  return 1;
};
// Milliseconds the store takes to save a relay key, so a test can poll while a key is being made.
let slowKeySave = 0;
const kvServer = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", async () => {
    const parsed = JSON.parse(body);
    if (slowKeySave && body.includes('"relay-key:')) await new Promise((r) => setTimeout(r, slowKeySave));
    const out = req.url === "/pipeline" || req.url === "/multi-exec" ? parsed.map((c) => ({ result: run(c) })) : { result: run(parsed) };
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(out));
  });
});

// Apple's sign-in service: whoever the test says signed in.
let sub = "001234.abcdef";
const idpServer = createServer((req, res) => {
  req.on("data", () => {});
  req.on("end", () => {
    const claims = { iss: "https://appleid.apple.com", aud: SERVICES_ID, exp: Math.floor(Date.now() / 1000) + 600, sub };
    const idToken = ["{}", JSON.stringify(claims), "sig"].map((p) => Buffer.from(p).toString("base64url")).join(".");
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ id_token: idToken }));
  });
});

const listen = (s) => new Promise((r) => s.listen(0, "127.0.0.1", () => r(`http://127.0.0.1:${s.address().port}`)));
let env;
before(async () => {
  env = {
    KV_REST_API_URL: await listen(kvServer),
    KV_REST_API_TOKEN: "kv-secret",
    APPLE_ID_HOST: await listen(idpServer),
    APPLE_SERVICES_ID: SERVICES_ID,
    APPLE_TEAM_ID: "TEAM123456",
    APPLE_SIGNIN_KEY_ID: "SIGNINKEY1",
    APPLE_SIGNIN_KEY_P8: pem(signIn),
    PORTAL_URL: "https://portal.example.com/",
    SESSION_SECRET: "a-long-random-secret",
  };
});
after(() => [kvServer, idpServer].forEach((s) => s.close()));
beforeEach(() => {
  store.clear();
  sets.clear();
  clock = 0;
  slowKeySave = 0;
  sub = "001234.abcdef";
});

const get = (path, cookies = "") => new Request(`https://portal.example.com${path}`, { headers: { cookie: cookies } });
const cookiesOf = (res) => res.headers.getSetCookie().map((c) => c.split(";")[0]).filter((c) => !c.endsWith("="));
const post = (path, form, cookies = "") => new Request(`https://portal.example.com${path}`, { method: "POST", headers: { cookie: cookies }, body: new URLSearchParams(form) });
const asLoft = (path, body, ip = "203.0.113.7") => new Request(`https://relay.example.com${path}`, { method: "POST", headers: { "x-forwarded-for": ip }, body: JSON.stringify(body) });
const secrets = new Map();
const newCode = async (ip) => {
  const { code, secret } = await (await pair(asLoft("/api/pair", {}, ip), env, clock)).json();
  secrets.set(code, secret);
  return code;
};
const pollFor = (code, secret = secrets.get(code)) => poll(asLoft("/api/pair-poll", { code, secret }), env);

/** Starts a sign-in and comes back from Apple; `before` are cookies the browser already holds. Returns the cookies. */
async function signInAs(before = "") {
  const started = await start(get("/auth/start"), env);
  const state = new URL(started.headers.get("location")).searchParams.get("state");
  const done = await callback(get(`/auth/callback?code=c0de&state=${state}`, cookiesOf(started).join("; ")), env);
  return { done, cookies: [before, ...cookiesOf(done)].filter(Boolean).join("; ") };
}

test("the code flow: a code, approved by a signed-in user, gives the Loft that asked its key once", async () => {
  const code = await newCode();
  assert.equal((await pollFor(code)).status, 202, "waiting while nobody approved");
  // Signed out, the page offers Sign in and remembers the code; after Apple, the portal sends the user back to it.
  const out = await connect(get(`/connect?code=${code}`), env);
  assert.match(await out.text(), /Sign in with Apple/);
  const remembered = cookiesOf(out).join("; ");
  const { done, cookies } = await signInAs(remembered);
  const back = await home(get("/", cookies), env);
  assert.equal(back.headers.get("location"), `/connect?code=${code}`);
  assert.match(await (await connect(get(`/connect?code=${code}`, cookies), env)).text(), /Allow/);
  const ok = await approve(post("/connect", { code, name: "Studio Mac" }, cookies), env);
  assert.equal(ok.status, 200);
  assert.match(await ok.text(), /Loft is signed in/);
  // The key comes once; then the code is dead.
  const got = await pollFor(code);
  assert.equal(got.status, 200);
  const { key } = await got.json();
  assert.match(key, /^lrk_[A-Za-z0-9_-]{43}$/);
  assert.equal((await pollFor(code)).status, 404);
  assert.equal((await approve(post("/connect", { code, name: "Again" }, cookies), env)).status, 404, "a used code can't be approved again");
  // The key is the account's: stored hashed, and the relay takes it.
  const hash = createHash("sha256").update(key).digest("hex");
  assert.equal(JSON.parse(store.get(`relay-key:${hash}`).value).account, "001234.abcdef");
  assert.ok([...store.keys()].every((k) => !store.get(k).value.includes?.(key)), "the key waits nowhere once collected");
  const pushed = await push(new Request("https://relay.example.com/api/push", { method: "POST", headers: { authorization: `Bearer ${key}` }, body: "{}" }), { ...env, APNS_KEY_P8: "x", APNS_KEY_ID: "x", APNS_TEAM_ID: "x", LOFT_BUNDLE_ID: "x" });
  assert.equal(pushed.status, 400, "past the key check (the body is what's wrong)");
});

test("an unapproved or expired code returns no key", async () => {
  const code = await newCode();
  assert.equal((await (await pollFor(code)).json()).key, undefined);
  clock += 601_000;
  assert.equal((await pollFor(code)).status, 404);
  assert.equal((await connect(get(`/connect?code=${code}`), env)).status, 404);
  const { cookies } = await signInAs();
  assert.equal((await approve(post("/connect", { code, name: "Late" }, cookies), env)).status, 404);
  assert.equal([...store.keys()].filter((k) => k.startsWith("relay-key:")).length, 0, "no key was made");
  // Made-up codes, and anything not shaped like one, get nothing either.
  for (const bad of ["", "x", "A".repeat(22), 5, null]) assert.equal((await pollFor(bad)).status, 404, String(bad));
});

test("approving needs a signed-in user and a name; two approvals make one key", async () => {
  const code = await newCode();
  assert.equal((await approve(post("/connect", { code, name: "Mac" }), env)).status, 303, "signed out goes to the portal");
  const { cookies } = await signInAs();
  assert.equal((await approve(post("/connect", { code, name: "  " }, cookies), env)).status, 400);
  assert.equal((await pollFor(code)).status, 202, "still waiting after the refusals");
  const both = await Promise.all([approve(post("/connect", { code, name: "A" }, cookies), env), approve(post("/connect", { code, name: "B" }, cookies), env)]);
  assert.deepEqual(both.map((r) => r.status).sort(), [200, 404]);
  assert.equal([...store.keys()].filter((k) => k.startsWith("relay-key:")).length, 1);
});

test("an account at its key cap gets no key from a code, and the code stays good for a retry", async () => {
  const { cookies } = await signInAs();
  for (let i = 0; i < 5; i++) assert.equal((await approve(post("/connect", { code: await newCode(`10.0.0.${i}`), name: `K${i}` }, cookies), env)).status, 200);
  const code = await newCode("10.0.0.9");
  assert.equal((await approve(post("/connect", { code, name: "Sixth" }, cookies), env)).status, 409);
  assert.equal((await pollFor(code)).status, 202, "still waiting, not given a key");
  assert.equal([...store.keys()].filter((k) => k.startsWith("relay-key:")).length, 5);
  const [id] = sets.get("account-keys:001234.abcdef");
  store.delete(`relay-key:${id}`);
  sets.get("account-keys:001234.abcdef").delete(id);
  assert.equal((await approve(post("/connect", { code, name: "Sixth" }, cookies), env)).status, 200, "after a revoke it goes through");
});

test("a poll while the key is being made waits; it doesn't lose the code", async () => {
  const code = await newCode();
  const { cookies } = await signInAs();
  slowKeySave = 100;
  const approving = approve(post("/connect", { code, name: "Mac" }, cookies), env);
  await new Promise((r) => setTimeout(r, 30));
  assert.equal((await pollFor(code)).status, 202, "not 404 while the key is saved");
  assert.equal((await approving).status, 200);
  assert.equal((await pollFor(code)).status, 200, "the key arrives");
});

test("the code in the link isn't enough to collect the key", async () => {
  const code = await newCode();
  const { cookies } = await signInAs();
  assert.equal((await pollFor(code, null)).status, 404, "no secret");
  assert.equal((await pollFor(code, "A".repeat(22))).status, 404, "a wrong secret");
  assert.equal((await approve(post("/connect", { code, name: "Mac" }, cookies), env)).status, 200);
  assert.equal((await pollFor(code, "A".repeat(22))).status, 404, "a wrong secret after Allow");
  assert.equal((await pollFor(code)).status, 200, "the Loft's own secret gets the key");
});

test("asking for codes is limited per IP", async () => {
  for (let i = 0; i < 5; i++) assert.equal((await pair(asLoft("/api/pair", {}), env, clock)).status, 200);
  assert.equal((await pair(asLoft("/api/pair", {}), env, clock)).status, 429);
  assert.equal((await pair(asLoft("/api/pair", {}, "198.51.100.1"), env, clock)).status, 200, "another IP is unaffected");
  assert.ok([...store.keys()].every((k) => !k.includes("203.0.113.7")), "the store never holds an IP");
});
