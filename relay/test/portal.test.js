import { test, before, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { createHash, createPublicKey, generateKeyPairSync, verify } from "node:crypto";
import { createServer as createH2 } from "node:http2";
import { home, start, callback, createAccountKey, revokeAccountKey, confirmDelete, startDelete, deleteAccount } from "../lib/portal.js";
import { kv as makeKv } from "../lib/kv.js";
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
const expiries = new Map();
let failing = null; // a command the store answers with an error
const members = (key) => sets.get(key) ?? new Set();
const run = ([cmd, key, ...args]) => {
  if (cmd === "SET" && args[1] === "EX") expiries.set(key, args[2]);
  if (cmd === "SET") return args[1] === "NX" && store.has(key) ? null : (store.set(key, args[0]), "OK");
  if (cmd === "GET") return store.get(key) ?? null;
  if (cmd === "DEL") return [key, ...args].filter((k) => store.delete(k) || sets.delete(k)).length;
  if (cmd === "SADD") return (sets.set(key, members(key).add(args[0])), 1);
  if (cmd === "SREM") return args.filter((m) => members(key).delete(m)).length;
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
    const answer = (c) => (c[0] === failing ? { error: "ERR fake failure" } : { result: run(c) });
    const out = req.url === "/" ? answer(parsed) : parsed.map(answer);
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(out));
  });
});

// Apple's sign-in service: answers the token request with tokens for whoever the test says signed in, and takes
// revocations (`/auth/revoke`), answering `idp.revokeStatus`.
let idp = { sub: "001234.abcdef", status: 200 };
const tokenRequests = [];
const revoked = [];
let signIns = 0;
const idpServer = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", () => {
    if (req.url === "/auth/revoke") return (revoked.push(new URLSearchParams(body)), res.writeHead(idp.revokeStatus ?? 200).end());
    tokenRequests.push({ url: req.url, form: new URLSearchParams(body) });
    if (idp.status !== 200) return res.writeHead(idp.status).end("{}");
    signIns += 1;
    const claims = { iss: "https://appleid.apple.com", aud: SERVICES_ID, exp: Math.floor(Date.now() / 1000) + 600, sub: idp.sub, ...idp.claims };
    const idToken = ["{}", JSON.stringify(claims), "sig"].map((p) => Buffer.from(p).toString("base64url")).join(".");
    res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ id_token: idToken, access_token: `at-${signIns}`, refresh_token: `rt-${signIns}` }));
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
  expiries.clear();
  failing = null;
  tokenRequests.length = 0;
  revoked.length = 0;
  pushed.length = 0;
  idp = { sub: "001234.abcdef", status: 200 };
});

// Each request comes from its own address unless a test gives one, so only the limits' own tests reach them.
let addresses = 0;
const fresh = () => `198.51.100.${addresses++ % 250}, 10.0.0.1`;
const get = (path, cookies = "", ip = fresh()) => new Request(`https://portal.example.com${path}`, { headers: { cookie: cookies, "x-forwarded-for": ip } });
const cookiesOf = (res) => res.headers.getSetCookie().map((c) => c.split(";")[0]).filter((c) => !c.endsWith("="));
const post = (path, form, cookies, ip = fresh()) => new Request(`https://portal.example.com${path}`, { method: "POST", headers: { cookie: cookies, "x-forwarded-for": ip, "sec-fetch-site": "same-origin" }, body: new URLSearchParams(form) });
/** The store's keys but the limits' counters (which hold hashes of addresses only). */
const kept = () => [...store.keys()].filter((k) => !k.startsWith("limit:"));

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

test("a PORTAL_URL that isn't an https:// URL with a host is not configured: it says why, without the value, and never goes to Apple", async () => {
  for (const bad of ["portal.get-loft.com", "http://portal.get-loft.com", "https://", "https:// x"]) {
    const bad_env = { ...env, PORTAL_URL: bad };
    for (const res of [await home(get("/"), bad_env), await start(get("/auth/start"), bad_env)]) {
      assert.equal(res.status, 503, bad);
      assert.equal(res.headers.get("location"), null, bad);
      const text = await res.text();
      assert.match(text, /PORTAL_URL must start with https:\/\//);
      assert.ok(!text.includes(bad.trim()) || bad === "https://", bad);
    }
  }
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
  assert.deepEqual(kept(), ["account:001234.abcdef"]);
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
  assert.deepEqual(kept(), []);
});

test("the sign-in state cookie is not a session", async () => {
  const started = await start(get("/auth/start"), env);
  const asSession = cookiesOf(started).join("; ").replace(/^__Host-state=/, "__Host-session=");
  assert.match(await (await home(get("/", asSession), env)).text(), /Sign in with Apple/);
  const res = await createAccountKey(post("/keys", { name: "x" }, asSession), env);
  assert.equal(res.status, 303);
  assert.equal(res.headers.get("location"), "/");
  assert.deepEqual(kept(), []);
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
  assert.deepEqual(transactions, [["EXISTS", "SET", "SADD", "SCARD"]], "the key and its place in the account are written and counted together");
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
  assert.match(res.headers.getSetCookie().join(), /__Host-session=; Max-Age=0/);
  assert.ok(![...store.keys()].some((k) => k.startsWith("relay-key:")));
});

test("two keys created at once at 4 can't both be the 5th", async () => {
  const session = await signInAs();
  for (let i = 1; i <= 4; i++) await createAccountKey(post("/keys", { name: `Mac ${i}` }, session), env);
  const both = await Promise.all(["A", "B"].map((name) => createAccountKey(post("/keys", { name }, session), env)));
  assert.deepEqual(both.map((r) => r.status).sort(), [200, 409]);
  assert.equal(sets.get("account-keys:001234.abcdef").size, 5);
  assert.equal([...store.keys()].filter((k) => k.startsWith("relay-key:")).length, 5, "the refused one's record is undone");
});

test("a key deleted by hand neither shows nor counts toward the 5", async () => {
  const session = await signInAs();
  for (let i = 1; i <= 5; i++) await createAccountKey(post("/keys", { name: `Mac ${i}` }, session), env);
  const [ghost] = idsOn(await (await home(get("/", session), env)).text());
  store.delete(`relay-key:${ghost}`); // the README's runbook
  const html = await (await home(get("/", session), env)).text();
  assert.ok(!idsOn(html).includes(ghost));
  assert.match(html, /4 of 5 keys/);
  assert.ok(!sets.get("account-keys:001234.abcdef").has(ghost), "dropped from the account's set");
  assert.equal((await createAccountKey(post("/keys", { name: "Mac 6" }, session), env)).status, 200);
});

test("a key's last-used date expires, so a push racing a revoke can't keep one forever", async () => {
  const session = await signInAs();
  const key = await keyFrom(await createAccountKey(post("/keys", { name: "Mac" }, session), env));
  await pushWith(key);
  assert.equal(expiries.get(`relay-key-used:${sha256(key)}`), 365 * 86400);
  // A key without one that's over a year old may have been used before then.
  const old = await signInAs("999.old");
  await createAccountKey(post("/keys", { name: "Old" }, old), env, Date.UTC(2025, 0, 1));
  assert.match(await (await home(get("/", old), env)).text(), /<td>Old<\/td><td>Jan 1, 2025<\/td><td>not in the last year<\/td>/);
});

test("a store error is an error, not an empty answer", async () => {
  const session = await signInAs();
  failing = "EXISTS";
  await assert.rejects(createAccountKey(post("/keys", { name: "Mac" }, session), env), /store refused a command/);
  failing = "SISMEMBER";
  await assert.rejects(revokeAccountKey(post("/keys/revoke", { id: "a".repeat(64) }, session), env), /store refused a command/);
});

/** Presses Delete my account and comes back from Apple signed in as `sub`. */
async function deleteAs(session, sub = "001234.abcdef") {
  const started = await startDelete(post("/account/delete", {}, session), env);
  assert.equal(started.status, 303);
  idp.sub = sub;
  const state = new URL(started.headers.get("location")).searchParams.get("state");
  return callback(get(`/auth/callback?code=c0de&state=${state}`, [session, ...cookiesOf(started)].join("; ")), env);
}

test("Delete my account deletes the keys and the account and revokes Apple's token", async () => {
  const session = await signInAs();
  const keys = [];
  for (const name of ["Mac", "MacBook", "iMac"]) keys.push(await keyFrom(await createAccountKey(post("/keys", { name }, session), env)));
  await pushWith(keys[0]);
  const other = await signInAs("999.other");
  const theirs = await keyFrom(await createAccountKey(post("/keys", { name: "Theirs" }, other), env));
  assert.match(await (await home(get("/", session), env)).text(), /href="\/account\/delete"/);
  assert.match(await (await confirmDelete(get("/account/delete", session), env)).text(), /deletes your account and all 3 keys, so those Lofts stop pushing, and ends[\s\S]*sign in once more/);

  const res = await deleteAs(session);
  assert.equal(res.status, 200);
  assert.match(await res.text(), /Account deleted/);
  assert.match(res.headers.getSetCookie().join(), /__Host-session=; Max-Age=0/);
  for (const key of keys) assert.equal((await pushWith(key)).status, 401);
  assert.ok(![...store.keys(), ...sets.keys()].some((k) => k.includes("001234.abcdef")), "nothing of the account is left");
  assert.ok(!keys.some((k) => store.has(`relay-key:${sha256(k)}`) || store.has(`relay-key-used:${sha256(k)}`)));
  // Apple was asked to revoke the token from the confirming sign-in, with the portal's client secret.
  assert.equal(revoked.length, 1);
  assert.equal(revoked[0].get("token"), `rt-${signIns}`);
  assert.equal(revoked[0].get("token_type_hint"), "refresh_token");
  assert.equal(revoked[0].get("client_id"), SERVICES_ID);
  assert.ok(revoked[0].get("client_secret"));
  // The other account is untouched, and the session left over is signed out of making keys.
  assert.equal((await pushWith(theirs)).status, 200);
  assert.equal((await createAccountKey(post("/keys", { name: "Again" }, session), env)).status, 303);
});

test("only the account's own Apple ID can confirm deleting it", async () => {
  const session = await signInAs();
  const key = await keyFrom(await createAccountKey(post("/keys", { name: "Mac" }, session), env));
  const res = await deleteAs(session, "999.someone-else");
  assert.equal(res.status, 403);
  assert.equal((await pushWith(key)).status, 200);
  assert.ok(store.has("account:001234.abcdef"));
  assert.ok(!store.has("account:999.someone-else"), "and the other Apple ID isn't signed up either");
  assert.equal(revoked.length, 0);
});

test("deleting needs a signed-in session and a sign-in started for it", async () => {
  assert.equal((await startDelete(post("/account/delete", {}, ""), env)).headers.get("location"), "/");
  assert.equal((await confirmDelete(get("/account/delete", ""), env)).headers.get("location"), "/");
  const session = await signInAs();
  // Pressed, then abandoned at Apple: the next ordinary sign-in in that browser signs in and deletes nothing.
  const abandoned = await startDelete(post("/account/delete", {}, session), env);
  const intent = cookiesOf(abandoned).find((c) => c.startsWith("__Host-delete="));
  const started = await start(get("/auth/start"), env);
  const state = new URL(started.headers.get("location")).searchParams.get("state");
  const done = await callback(get(`/auth/callback?code=c0de&state=${state}`, [...cookiesOf(started).filter((c) => c.startsWith("__Host-state=")), intent].join("; ")), env);
  assert.equal(done.headers.get("location"), "/");
  assert.ok(store.has("account:001234.abcdef"));
  assert.equal(revoked.length, 0);
});

test("the delete page reads well with no key and with one", async () => {
  const session = await signInAs();
  assert.match(await (await confirmDelete(get("/account/delete", session), env)).text(), /This deletes your account, and ends Sign in with Apple/);
  await createAccountKey(post("/keys", { name: "Mac" }, session), env);
  assert.match(await (await confirmDelete(get("/account/delete", session), env)).text(), /This deletes your account and its key, so its Loft stops pushing, and ends/);
});

test("a delete that fails part way says so, keeps the keys reachable, and can be tried again", async () => {
  const session = await signInAs();
  const key = await keyFrom(await createAccountKey(post("/keys", { name: "Mac" }, session), env));
  failing = "SMEMBERS";
  const res = await deleteAs(session);
  assert.equal(res.status, 502);
  assert.match(await res.text(), /couldn't be deleted completely[\s\S]*href="\/account\/delete"/);
  assert.doesNotMatch(res.headers.getSetCookie().join(), /__Host-session=;/, "still signed in, to try again");
  assert.equal(revoked.length, 0);
  assert.equal((await pushWith(key)).status, 200);
  failing = null;
  assert.equal((await deleteAs(session)).status, 200);
  assert.equal((await pushWith(key)).status, 401);
});

test("a key made while the account is being deleted is deleted too", async () => {
  const session = await signInAs();
  const key = await keyFrom(await createAccountKey(post("/keys", { name: "Mac" }, session), env));
  const kv = makeKv(env);
  let late = null;
  // Between reading the keys and deleting them, another tab makes one.
  const racing = { ...kv, transaction: async (commands) => {
    if (!late) late = await keyFrom(await createAccountKey(post("/keys", { name: "Late" }, session), env));
    return kv.transaction(commands);
  } };
  await deleteAccount(racing, "001234.abcdef");
  for (const k of [key, late]) assert.equal((await pushWith(k)).status, 401);
  assert.ok(![...store.keys(), ...sets.keys()].some((k) => k.includes("001234.abcdef") || k.endsWith(sha256(late))), "nothing is left");
});

test("the account is deleted even when Apple doesn't confirm the revocation, and the page says so", async () => {
  const session = await signInAs();
  idp.revokeStatus = 500;
  const res = await deleteAs(session);
  assert.equal(res.status, 200);
  assert.match(await res.text(), /Apple didn't confirm ending the sign-in/);
  assert.ok(!store.has("account:001234.abcdef"));
});

test("the 6th sign-in from one address in an hour is refused, and the next hour it works again", async () => {
  const ip = "203.0.113.9, 10.0.0.1";
  const hour = Date.UTC(2026, 10, 6, 14, 20);
  for (let i = 1; i <= 5; i++) assert.equal((await start(get("/auth/start", "", ip), env, hour)).status, 303, `sign-in ${i}`);
  const refused = await start(get("/auth/start", "", ip), env, hour);
  assert.equal(refused.status, 429);
  assert.equal(refused.headers.get("retry-after"), String(40 * 60));
  assert.match(await refused.text(), /At most 5 sign-ins an hour from one network\. Try again in 40 minutes/);
  assert.equal(refused.headers.get("location"), null, "not sent to Apple");
  assert.equal((await start(get("/auth/start", "", "203.0.113.10"), env, hour)).status, 303, "another address");
  assert.equal((await start(get("/auth/start", "", ip), env, hour + 3600_000)).status, 303, "the next hour");
  // Pressing Delete my account is a sign-in too.
  const session = await signInAs();
  assert.equal((await startDelete(post("/account/delete", {}, session, ip), env, hour)).status, 429);
  assert.ok(![...store.keys()].some((k) => k.includes("203.0.113")), "the store holds hashes of addresses only");
});

test("the 6th sign-in coming back from Apple to one address in an hour is refused before Apple is asked", async () => {
  const ip = "203.0.113.11";
  for (let i = 1; i <= 5; i++) assert.equal((await callback(get("/auth/callback?code=x&state=y", "", ip), env)).status, 400);
  const asked = tokenRequests.length;
  assert.equal((await callback(get("/auth/callback?code=x&state=y", "", ip), env)).status, 429);
  assert.equal(tokenRequests.length, asked);
});

test("the 6th key creation from one address in an hour is refused, across accounts", async () => {
  const ip = "203.0.113.12";
  const [a, b] = [await signInAs("111.a"), await signInAs("222.b")];
  for (const [i, session] of [a, a, a, b, b].entries()) assert.ok(await keyFrom(await createAccountKey(post("/keys", { name: `Mac ${i}` }, session, ip), env)), `key ${i + 1}`);
  const refused = await createAccountKey(post("/keys", { name: "Mac 6" }, b, ip), env);
  assert.equal(refused.status, 429);
  assert.match(await refused.text(), /At most 5 key creations an hour/);
  assert.equal(sets.get("account-keys:222.b").size, 2);
  assert.ok(await keyFrom(await createAccountKey(post("/keys", { name: "Mac 6" }, b), env)), "from another address");
});

/** `post`, but as another site would send it: `Sec-Fetch-Site: cross-site`, a foreign `Origin`, or neither header. */
const forged = (req, headers) => {
  req.headers.delete("sec-fetch-site");
  for (const [k, v] of Object.entries(headers)) req.headers.set(k, v);
  return req;
};
const FORGERIES = [{ "sec-fetch-site": "cross-site" }, { "sec-fetch-site": "same-site" }, { origin: "https://evil.example.com" }, {}];

test("a form posted from another site is refused with 403 and changes nothing", async () => {
  const session = await signInAs();
  const key = await keyFrom(await createAccountKey(post("/keys", { name: "Mac" }, session), env));
  const before = JSON.stringify([...store]);
  const id = sha256(key);
  for (const headers of FORGERIES) {
    for (const [handler, path, form] of [[createAccountKey, "/keys", { name: "Evil" }], [revokeAccountKey, "/keys/revoke", { id }], [startDelete, "/account/delete", {}]]) {
      const res = await handler(forged(post(path, form, session), headers), env);
      assert.equal(res.status, 403, `${path} ${JSON.stringify(headers)}`);
      assert.match(await res.text(), /didn't come from this portal/);
      assert.equal(res.headers.get("location"), null);
    }
  }
  assert.equal(JSON.stringify([...store]), before, "nothing was written, not even a rate-limit count");
  assert.equal((await pushWith(key)).status, 200, "the key still works");
  // The portal's own origin passes, with or without Sec-Fetch-Site.
  assert.equal((await createAccountKey(forged(post("/keys", { name: "Own" }, session), { origin: "https://portal.example.com" }), env)).status, 200);
});

test("the session, state and delete cookies are __Host- cookies: Secure, HttpOnly, SameSite=Lax, Path=/, no Domain", async () => {
  const started = await start(get("/auth/start"), env);
  const state = new URL(started.headers.get("location")).searchParams.get("state");
  const done = await callback(get(`/auth/callback?code=c0de&state=${state}`, cookiesOf(started).join("; ")), env);
  for (const res of [started, done]) {
    for (const c of res.headers.getSetCookie()) {
      assert.match(c, /^__Host-(session|state|delete)=/);
      assert.match(c, /; Path=\/; HttpOnly; Secure; SameSite=Lax$/);
      assert.doesNotMatch(c, /Domain/i);
    }
  }
  assert.ok(done.headers.getSetCookie().some((c) => c.startsWith("__Host-session=") && !c.includes("Max-Age=0")));
  // Cookies under the old names are no longer read.
  const old = cookiesOf(done).join("; ").replaceAll("__Host-", "");
  assert.equal((await home(get("/", old), env)).status, 200);
  assert.doesNotMatch(await (await home(get("/", old), env)).text(), /Create key/);
});

const HTML_HEADERS = {
  "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; font-src 'self'; img-src 'self'; form-action 'self' https://appleid.apple.com; base-uri 'none'; frame-ancestors 'none'",
  "referrer-policy": "no-referrer",
  "x-content-type-options": "nosniff",
};

test("every portal page carries the CSP (no scripts), Referrer-Policy and nosniff; the push endpoint carries nosniff and no-store", async () => {
  const session = await signInAs();
  const pages = [
    await home(get("/"), env),
    await home(get("/", session), env),
    await confirmDelete(get("/account/delete", session), env),
    await createAccountKey(post("/keys", { name: "Mac" }, session), env),
    await createAccountKey(post("/keys", { name: " " }, session), env),
    await revokeAccountKey(post("/keys/revoke", { id: "x" }, session), env),
    await createAccountKey(forged(post("/keys", { name: "x" }, session), { "sec-fetch-site": "cross-site" }), env),
    await callback(get("/auth/callback?code=x&state=y"), env),
    await home(get("/"), {}),
  ];
  for (const res of pages) {
    assert.match(res.headers.get("content-type"), /^text\/html/);
    for (const [k, v] of Object.entries(HTML_HEADERS)) assert.equal(res.headers.get(k), v, `${res.status} ${k}`);
  }
  assert.doesNotMatch(HTML_HEADERS["content-security-policy"], /script/);
  const key = await keyFrom(await createAccountKey(post("/keys", { name: "Two" }, session), env));
  for (const res of [await pushWith(key), await pushWith("nope")]) {
    assert.equal(res.headers.get("x-content-type-options"), "nosniff");
    assert.equal(res.headers.get("cache-control"), "no-store");
  }
});
