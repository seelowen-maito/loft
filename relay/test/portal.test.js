import { test, before, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { createHash, createPublicKey, generateKeyPairSync, verify } from "node:crypto";
import { createServer as createH2 } from "node:http2";
import { home, start, callback, createAccountKey, revokeAccountKey } from "../lib/portal.js";
import { push } from "../lib/relay.js";

const SERVICES_ID = "com.example.loft.web";
const BUNDLE = "com.example.loft";
const DEVICE = "a1".repeat(32);
const signIn = generateKeyPairSync("ec", { namedCurve: "P-256" });
const apns = generateKeyPairSync("ec", { namedCurve: "P-256" });
const pem = (k) => k.privateKey.export({ type: "pkcs8", format: "pem" });
const sha256 = (s) => createHash("sha256").update(s).digest("hex");

// The store: Upstash's REST API, with its transactions (`/multi-exec`) recorded.
const store = new Map();
const sets = new Map();
const transactions = [];
const members = (key) => sets.get(key) ?? new Set();
const run = ([cmd, key, ...args]) => {
  if (cmd === "SET") return args[1] === "NX" && store.has(key) ? null : (store.set(key, args[0]), "OK");
  if (cmd === "GET") return store.get(key) ?? null;
  if (cmd === "DEL") return [key, ...args].filter((k) => store.delete(k)).length;
  if (cmd === "SADD") return (sets.set(key, members(key).add(args[0])), 1);
  if (cmd === "SREM") return members(key).delete(args[0]) ? 1 : 0;
  if (cmd === "SMEMBERS") return [...members(key)];
  if (cmd === "SCARD") return members(key).size;
  if (cmd === "SISMEMBER") return members(key).has(args[0]) ? 1 : 0;
  if (cmd === "EXISTS") return store.has(key) ? 1 : 0;
  if (cmd === "INCR") return (store.set(key, String(Number(store.get(key) ?? 0) + 1)), Number(store.get(key)));
  return 1;
};
const kvServer = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", () => {
    const parsed = JSON.parse(body);
    if (req.url === "/multi-exec") transactions.push(parsed.map((c) => c[0]));
    const out = req.url === "/" ? { result: run(parsed) } : parsed.map((c) => ({ result: run(c) }));
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(out));
  });
});

// Apple's sign-in service: answers the token request with an id token for whoever the test says signed in.
let idp = { sub: "001234.abcdef", status: 200 };
const tokenRequests = [];
const idpServer = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", () => {
    tokenRequests.push({ url: req.url, form: new URLSearchParams(body) });
    if (idp.status !== 200) return res.writeHead(idp.status).end("{}");
    const claims = { iss: "https://appleid.apple.com", aud: SERVICES_ID, exp: Math.floor(Date.now() / 1000) + 600, sub: idp.sub, ...idp.claims };
    const idToken = ["{}", JSON.stringify(claims), "sig"].map((p) => Buffer.from(p).toString("base64url")).join(".");
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ id_token: idToken }));
  });
});

// Apple's push service.
const pushed = [];
const apnsServer = createH2((req, res) => {
  req.on("data", () => {});
  req.on("end", () => (pushed.push(req.headers[":path"]), res.writeHead(200, { "apns-id": "apns-1" }), res.end()));
});

const listen = (s) => new Promise((r) => s.listen(0, "127.0.0.1", () => r(`http://127.0.0.1:${s.address().port}`)));
let env;
before(async () => {
  env = {
    KV_REST_API_URL: await listen(kvServer),
    KV_REST_API_TOKEN: "kv-secret",
    APNS_HOST_PRODUCTION: await listen(apnsServer),
    APNS_KEY_P8: pem(apns),
    APNS_KEY_ID: "ABC123DEFG",
    APNS_TEAM_ID: "TEAM123456",
    LOFT_BUNDLE_ID: BUNDLE,
    APPLE_ID_HOST: await listen(idpServer),
    APPLE_SERVICES_ID: SERVICES_ID,
    APPLE_TEAM_ID: "TEAM123456",
    APPLE_SIGNIN_KEY_ID: "SIGNINKEY1",
    APPLE_SIGNIN_KEY_P8: pem(signIn),
    PORTAL_URL: "https://portal.example.com/",
    SESSION_SECRET: "a-long-random-secret",
  };
});
after(() => [kvServer, idpServer, apnsServer].forEach((s) => s.close()));
beforeEach(() => {
  store.clear();
  sets.clear();
  transactions.length = 0;
  tokenRequests.length = 0;
  pushed.length = 0;
  idp = { sub: "001234.abcdef", status: 200 };
});

const get = (path, cookies = "") => new Request(`https://portal.example.com${path}`, { headers: { cookie: cookies } });
const cookiesOf = (res) => res.headers.getSetCookie().map((c) => c.split(";")[0]).filter((c) => !c.endsWith("="));
const post = (path, form, cookies) => new Request(`https://portal.example.com${path}`, { method: "POST", headers: { cookie: cookies }, body: new URLSearchParams(form) });

/** Starts a sign-in and comes back from Apple: the session cookie. */
async function signInAs(sub = "001234.abcdef") {
  idp.sub = sub;
  const started = await start(get("/auth/start"), env);
  const state = new URL(started.headers.get("location")).searchParams.get("state");
  const done = await callback(get(`/auth/callback?code=c0de&state=${state}`, cookiesOf(started).join("; ")), env);
  assert.equal(done.headers.get("location"), "/");
  return cookiesOf(done).join("; ");
}

test("without its Apple settings the portal says it isn't configured", async () => {
  for (const missing of ["APPLE_SERVICES_ID", "APPLE_SIGNIN_KEY_P8", "PORTAL_URL", "SESSION_SECRET"]) {
    const res = await home(get("/"), { ...env, [missing]: "" });
    assert.equal(res.status, 503, missing);
    assert.match(await res.text(), /isn't configured/);
  }
  assert.equal((await start(get("/auth/start"), { ...env, PORTAL_URL: "" })).status, 503);
});

test("signed out, the page offers Sign in with Apple, which asks Apple for no email or name", async () => {
  assert.match(await (await home(get("/"), env)).text(), /Sign in with Apple/);
  const res = await start(get("/auth/start"), env);
  assert.equal(res.status, 303);
  const to = new URL(res.headers.get("location"));
  assert.equal(to.pathname, "/auth/authorize");
  assert.equal(to.searchParams.get("client_id"), SERVICES_ID);
  assert.equal(to.searchParams.get("redirect_uri"), "https://portal.example.com/auth/callback");
  assert.equal(to.searchParams.get("response_type"), "code");
  assert.equal(to.searchParams.get("scope"), null, "no email or name scope");
});

test("signing in creates an account holding only Apple's user id", async () => {
  const session = await signInAs("001234.abcdef");
  assert.deepEqual([...store.keys()], ["account:001234.abcdef"]);
  assert.deepEqual(Object.keys(JSON.parse(store.get("account:001234.abcdef"))), ["created"]);
  assert.match(await (await home(get("/", session), env)).text(), /Create key/);
  // Apple got the code and a client secret signed with the sign-in key.
  const { form } = tokenRequests[0];
  assert.equal(form.get("code"), "c0de");
  assert.equal(form.get("client_id"), SERVICES_ID);
  const [h, c, s] = form.get("client_secret").split(".");
  assert.deepEqual(JSON.parse(Buffer.from(h, "base64url")), { alg: "ES256", kid: "SIGNINKEY1" });
  assert.equal(JSON.parse(Buffer.from(c, "base64url")).sub, SERVICES_ID);
  assert.ok(verify("sha256", Buffer.from(`${h}.${c}`), { key: createPublicKey(signIn.privateKey), dsaEncoding: "ieee-p1363" }, Buffer.from(s, "base64url")));
});

test("signing in again keeps the same account", async () => {
  await signInAs();
  const created = store.get("account:001234.abcdef");
  await signInAs();
  assert.equal(store.get("account:001234.abcdef"), created);
});

test("a sign-in that Apple didn't start for this browser, or can't confirm, creates no account", async () => {
  const started = await start(get("/auth/start"), env);
  const state = new URL(started.headers.get("location")).searchParams.get("state");
  const mine = cookiesOf(started).join("; ");
  for (const [url, cookies] of [[`/auth/callback?code=x&state=other`, mine], [`/auth/callback?code=x&state=${state}`, ""], [`/auth/callback?state=${state}`, mine]]) {
    assert.equal((await callback(get(url, cookies), env)).status, 400, url);
  }
  for (const bad of [{ status: 400 }, { claims: { aud: "someone.else" } }, { claims: { iss: "https://evil.example" } }, { claims: { exp: 1 } }]) {
    idp = { sub: "x", status: 200, ...bad };
    assert.equal((await callback(get(`/auth/callback?code=x&state=${state}`, mine), env)).status, 502, JSON.stringify(bad));
  }
  assert.equal(store.size, 0);
});

test("the sign-in state cookie is not a session", async () => {
  const started = await start(get("/auth/start"), env);
  const asSession = cookiesOf(started).join("; ").replace(/^state=/, "session=");
  assert.match(await (await home(get("/", asSession), env)).text(), /Sign in with Apple/);
  const res = await createAccountKey(post("/keys", { name: "x" }, asSession), env);
  assert.equal(res.status, 303);
  assert.equal(res.headers.get("location"), "/");
  assert.equal(store.size, 0);
});

test("a forged or expired session is signed out", async () => {
  const session = await signInAs();
  const forged = session.replace(/\.[^.]+$/, ".AAAA");
  assert.match(await (await home(get("/", forged), env)).text(), /Sign in with Apple/);
  assert.match(await (await home(get("/", session), env, Date.now() + 8 * 86400_000)).text(), /Sign in with Apple/);
  assert.equal((await createAccountKey(post("/keys", { name: "x" }, forged), env)).status, 303);
});

test("a key is shown once and only its hash is stored", async () => {
  const session = await signInAs();
  const res = await createAccountKey(post("/keys", { name: "My <Mac>" }, session), env, Date.UTC(2026, 10, 5));
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "no-store");
  const key = (await res.text()).match(/lrk_[A-Za-z0-9_-]{43}/)[0];
  const record = store.get(`relay-key:${sha256(key)}`);
  assert.deepEqual(JSON.parse(record), { label: "My <Mac>", created: "2026-11-05T00:00:00.000Z", account: "001234.abcdef" });
  assert.deepEqual([...sets.get("account-keys:001234.abcdef")], [sha256(key)]);
  assert.deepEqual(transactions, [["SET", "SADD"]], "the key and its place in the account are written together");
  assert.ok(![...store.keys(), ...store.values(), ...[...sets.values()].flatMap((m) => [...m])].some((v) => v.includes(key)), "the key itself isn't stored");
  assert.ok(!(await (await home(get("/", session), env)).text()).includes(key), "it isn't shown again");
  assert.equal((await createAccountKey(post("/keys", { name: " " }, session), env)).status, 400, "a key needs a name");
});

test("a push with a portal key is forwarded; an unknown key is refused", async () => {
  const session = await signInAs();
  const key = (await (await createAccountKey(post("/keys", { name: "Mac" }, session), env)).text()).match(/lrk_[A-Za-z0-9_-]{43}/)[0];
  const send = (token) =>
    push(new Request("http://relay/api/push", { method: "POST", headers: { authorization: `Bearer ${token}` }, body: JSON.stringify({ device_token: DEVICE, environment: "production", bundle_id: BUNDLE, payload: { aps: { alert: "hi" } } }) }), env);
  assert.equal((await send(key)).status, 200);
  assert.deepEqual(pushed, [`/3/device/${DEVICE}`]);
  assert.equal((await send(`lrk_${"b".repeat(43)}`)).status, 401);
  assert.equal(pushed.length, 1);
});

const keyFrom = async (res) => (await res.text()).match(/lrk_[A-Za-z0-9_-]{43}/)?.[0];
const pushWith = (key) =>
  push(new Request("http://relay/api/push", { method: "POST", headers: { authorization: `Bearer ${key}` }, body: JSON.stringify({ device_token: DEVICE, environment: "production", bundle_id: BUNDLE, payload: { aps: { alert: "hi" } } }) }), env, Date.UTC(2026, 10, 6, 12));
const idsOn = (html) => [...html.matchAll(/name="id" value="([0-9a-f]{64})"/g)].map((m) => m[1]);

test("the key list shows each key's name, created and last-used dates, never the key", async () => {
  const session = await signInAs();
  const mini = await keyFrom(await createAccountKey(post("/keys", { name: "Mac <mini>" }, session), env, Date.UTC(2026, 10, 3)));
  const book = await keyFrom(await createAccountKey(post("/keys", { name: "MacBook" }, session), env, Date.UTC(2026, 10, 4)));
  assert.equal((await pushWith(mini)).status, 200);
  const html = await (await home(get("/", session), env)).text();
  assert.match(html, /<td>Mac &lt;mini&gt;<\/td><td>Nov 3, 2026<\/td><td>Nov 6, 2026<\/td>/);
  assert.match(html, /<td>MacBook<\/td><td>Nov 4, 2026<\/td><td>never<\/td>/);
  assert.match(html, /2 of 5 keys/);
  assert.ok(!html.includes(mini) && !html.includes(book), "no key on the page");
  // Another account sees none of them.
  assert.match(await (await home(get("/", await signInAs("999.other")), env)).text(), /No keys yet[\s\S]*0 of 5 keys/);
});

test("a revoked key stops working at once, and only its owner can revoke it", async () => {
  const session = await signInAs();
  const key = await keyFrom(await createAccountKey(post("/keys", { name: "Old iMac" }, session), env));
  assert.equal((await pushWith(key)).status, 200);
  const [id] = idsOn(await (await home(get("/", session), env)).text());
  assert.equal(id, sha256(key));
  const other = await signInAs("999.other");
  for (const [form, cookies] of [[{ id }, other], [{ id: "x" }, session], [{}, session]]) {
    assert.equal((await revokeAccountKey(post("/keys/revoke", form, cookies), env)).status, 404, JSON.stringify(form));
  }
  assert.equal((await revokeAccountKey(post("/keys/revoke", { id }, ""), env)).status, 303, "signed out goes home");
  assert.equal((await pushWith(key)).status, 200, "still works after the refused attempts");
  const res = await revokeAccountKey(post("/keys/revoke", { id }, session), env);
  assert.equal(res.status, 303);
  assert.equal((await pushWith(key)).status, 401);
  assert.ok(!store.has(`relay-key:${id}`) && !store.has(`relay-key-used:${id}`), "the key's record and last-used date are gone");
  assert.equal(sets.get("account-keys:001234.abcdef").size, 0);
  assert.deepEqual(idsOn(await (await home(get("/", session), env)).text()), []);
});

test("a 6th key is refused with a clear message until one is revoked", async () => {
  const session = await signInAs();
  for (let i = 1; i <= 5; i++) assert.ok(await keyFrom(await createAccountKey(post("/keys", { name: `Mac ${i}` }, session), env)));
  const page = await (await home(get("/", session), env)).text();
  assert.match(page, /5 of 5 keys/);
  assert.ok(!page.includes("Create key"), "the form is gone at the cap");
  const refused = await createAccountKey(post("/keys", { name: "Mac 6" }, session), env);
  assert.equal(refused.status, 409);
  const text = await refused.text();
  assert.match(text, /at most 5 keys\. Revoke one to make room/);
  assert.equal(await keyFrom(new Response(text)), undefined);
  assert.equal(sets.get("account-keys:001234.abcdef").size, 5);
  await revokeAccountKey(post("/keys/revoke", { id: idsOn(page)[0] }, session), env);
  assert.ok(await keyFrom(await createAccountKey(post("/keys", { name: "Mac 6" }, session), env)));
});

test("a session whose account is gone gets no key and is signed out", async () => {
  const session = await signInAs();
  store.delete("account:001234.abcdef");
  const res = await createAccountKey(post("/keys", { name: "Mac" }, session), env);
  assert.equal(res.status, 303);
  assert.match(res.headers.getSetCookie().join(), /session=; Max-Age=0/);
  assert.ok(![...store.keys()].some((k) => k.startsWith("relay-key:")));
});
