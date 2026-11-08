// The push portal (ADR 0003): a web page where a user signs in with Apple and creates a relay key. It stores only
// Apple's stable user id and hashed keys; no email or name is asked for. The functions in api/ call one handler each.
// Every Apple value comes from the environment (a self-hoster sets their own); with any missing the portal says so.

import { createHash, createHmac, createPrivateKey, randomBytes, sign, timingSafeEqual } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { newKey, MAX_KEYS, USED_SECONDS } from "./relay.js";

export const REQUIRED = ["APPLE_SERVICES_ID", "APPLE_TEAM_ID", "APPLE_SIGNIN_KEY_ID", "APPLE_SIGNIN_KEY_P8", "PORTAL_URL", "SESSION_SECRET", "KV_REST_API_URL", "KV_REST_API_TOKEN"];
const APPLE = "https://appleid.apple.com";
const SESSION_SECONDS = 7 * 86400;
const STATE_SECONDS = 600;
/** Sign-ins, and key creations, one IP can make per hour. */
export const PER_HOUR = 5;

/** Apple's sign-in host; APPLE_ID_HOST replaces it for the tests' fake Apple, never in production. */
const appleHost = (env) => (env.VERCEL_ENV !== "production" && env.APPLE_ID_HOST) || APPLE;
const origin = (env) => env.PORTAL_URL.replace(/\/+$/, "");

export const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
// Loft's look (frontend/src/index.css): ink surfaces, hairlines, the green accent, Geist served from /fonts (no third-party request).
const STYLE = `@font-face{font-family:Geist;src:url(/fonts/Geist-Variable.woff2) format("woff2");font-weight:100 900;font-display:swap}@font-face{font-family:"Geist Mono";src:url(/fonts/GeistMono-Variable.woff2) format("woff2");font-weight:100 900;font-display:swap}` +
  `:root{color-scheme:dark;--ink-950:#08090a;--ink-900:#0d0e10;--ink-850:#121316;--ink-800:#17191c;--ink-700:#212328;--line:rgb(255 255 255/.07);--line-strong:rgb(255 255 255/.12);--fg:#eceae4;--fg-2:#a9a8a3;--fg-3:#6e6e6b;--accent:#c8f26d;--bad:#f27d6d}` +
  `*{box-sizing:border-box}body{margin:0;min-height:100vh;display:flex;flex-direction:column;align-items:center;padding:clamp(1.5rem,8vh,4.5rem) 1rem 2rem;background:var(--ink-950);color:var(--fg);font:15px/1.55 Geist,ui-sans-serif,system-ui,sans-serif;-webkit-font-smoothing:antialiased}` +
  `::selection{background:rgb(200 242 109/.3)}.brand{display:flex;align-items:center;gap:.6rem;margin-bottom:1.25rem;font-size:17px;font-weight:600;letter-spacing:-.01em}.brand .mark{border-radius:8px}.brand small{font:400 10.5px "Geist Mono",ui-monospace,monospace;text-transform:uppercase;letter-spacing:.14em;color:var(--fg-3)}` +
  `main{width:100%;max-width:34rem;padding:1.75rem;background:rgb(13 14 16/.7);border:1px solid var(--line);border-radius:1rem}h1{margin:0 0 1rem;font-size:22px;line-height:1.25;font-weight:600;letter-spacing:-.015em}p{margin:0 0 1rem;color:var(--fg-2)}main>:last-child{margin-bottom:0}` +
  `a{color:var(--fg);text-decoration-color:var(--line-strong);text-underline-offset:3px}a:hover{text-decoration-color:var(--accent)}form{margin:0 0 1rem}label{display:block;color:var(--fg-2);font-size:13px}` +
  `input:not([type=hidden]){display:block;width:100%;margin-top:.4rem;padding:.55rem .75rem;font:inherit;font-size:14px;color:var(--fg);background:var(--ink-850);border:1px solid var(--line-strong);border-radius:.75rem;outline:0}input::placeholder{color:var(--fg-3)}input:focus{border-color:rgb(200 242 109/.5);box-shadow:0 0 0 3px rgb(200 242 109/.1)}` +
  `button{font:inherit;font-size:13px;padding:.45rem .95rem;color:var(--fg);background:var(--ink-800);border:1px solid var(--line-strong);border-radius:999px;cursor:pointer}button:hover{background:var(--ink-700);border-color:rgb(255 255 255/.2)}button.primary{color:var(--ink-950);font-weight:500;background:var(--accent);border-color:var(--accent)}button.primary:hover{filter:brightness(1.1)}button.danger{color:var(--bad);border-color:rgb(242 125 109/.4);background:transparent}p.bad{color:var(--bad)}button.danger:hover{background:rgb(242 125 109/.1)}` +
  `.apple{display:inline-flex;align-items:center;justify-content:center;gap:.5rem;min-width:15rem;height:44px;padding:0 1.25rem;color:#000;background:#fff;border:1px solid #fff;border-radius:8px;font-size:17px;font-weight:500;text-decoration:none;white-space:nowrap}.apple:hover{background:#eee}.apple svg{height:20px;width:auto;fill:#000}` +
  `code{display:block;word-break:break-all;margin:0 0 1rem;padding:.8rem .9rem;font:13px/1.5 "Geist Mono",ui-monospace,"SF Mono",monospace;color:var(--accent);background:var(--ink-850);border:1px solid var(--line-strong);border-radius:.75rem;user-select:all}` +
  `.table{overflow-x:auto;margin:0 0 1rem}table{width:100%;border-collapse:collapse;font-size:13.5px}th{padding:.4rem .5rem .4rem 0;text-align:left;font:400 10.5px "Geist Mono",ui-monospace,monospace;text-transform:uppercase;letter-spacing:.14em;color:var(--fg-3);white-space:nowrap}td{padding:.55rem .5rem .55rem 0;text-align:left;border-top:1px solid var(--line);white-space:nowrap}td:last-child,th:last-child{padding-right:0;text-align:right}td form{margin:0}td button{padding:.25rem .7rem;font-size:12px}` +
  `.row{display:flex;flex-wrap:wrap;align-items:center;gap:.9rem}`;
const APPLE_LOGO = '<svg viewBox="0 0 814 1000" aria-hidden="true"><path d="M788.1 340.9c-5.8 4.5-108.2 62.2-108.2 190.5 0 148.4 130.3 200.9 134.2 202.2-.6 3.2-20.7 71.9-68.7 141.9-42.8 61.6-87.5 123.1-155.5 123.1s-85.5-39.5-164-39.5c-76.5 0-103.7 40.8-165.9 40.8s-105.6-57-155.5-127C46.7 790.7 0 663 0 541.8c0-194.4 126.4-297.5 250.8-297.5 66.1 0 121.2 43.4 162.7 43.4 39.5 0 101.1-46 176.3-46 28.5 0 130.9 2.6 198.3 99.2zm-234-181.5c31.1-36.9 53.1-88.1 53.1-139.3 0-7.1-.6-14.3-1.9-20.1-50.6 1.9-110.8 33.7-147.1 75.8-28.5 32.4-55.1 83.6-55.1 135.5 0 7.8 1.3 15.6 1.9 18.1 3.2.6 8.4 1.3 13.6 1.3 45.4 0 102.5-30.4 135.5-71.3z"/></svg>';
/** Apple's white "Sign in with Apple" button (their guidelines: white on dark, logo left of the label). */
export const appleButton = '<a class="apple" href="/auth/start">' + APPLE_LOGO + "Sign in with Apple</a>";
const LOGO = `<svg class="mark" width="28" height="28" aria-hidden="true" viewBox="0 0 16 16" shape-rendering="crispEdges"><rect width="16" height="16" rx="3.5" fill="#08090a"/><g fill="#c8f26d"><rect x="7" y="4" width="1" height="1"/><rect x="8" y="4" width="1" height="1"/><rect x="6" y="5" width="1" height="1"/><rect x="7" y="5" width="1" height="1"/><rect x="8" y="5" width="1" height="1"/><rect x="9" y="5" width="1" height="1"/><rect x="5" y="6" width="1" height="1"/><rect x="6" y="6" width="1" height="1"/><rect x="7" y="6" width="1" height="1"/><rect x="8" y="6" width="1" height="1"/><rect x="9" y="6" width="1" height="1"/><rect x="10" y="6" width="1" height="1"/><rect x="4" y="7" width="1" height="1"/><rect x="5" y="7" width="1" height="1"/><rect x="6" y="7" width="1" height="1"/><rect x="7" y="7" width="1" height="1"/><rect x="8" y="7" width="1" height="1"/><rect x="9" y="7" width="1" height="1"/><rect x="10" y="7" width="1" height="1"/><rect x="11" y="7" width="1" height="1"/><rect x="5" y="8" width="1" height="1"/><rect x="10" y="8" width="1" height="1"/><rect x="5" y="9" width="1" height="1"/><rect x="7" y="9" width="1" height="1"/><rect x="8" y="9" width="1" height="1"/><rect x="10" y="9" width="1" height="1"/><rect x="5" y="10" width="1" height="1"/><rect x="7" y="10" width="1" height="1"/><rect x="8" y="10" width="1" height="1"/><rect x="10" y="10" width="1" height="1"/><rect x="5" y="11" width="1" height="1"/><rect x="6" y="11" width="1" height="1"/><rect x="7" y="11" width="1" height="1"/><rect x="8" y="11" width="1" height="1"/><rect x="9" y="11" width="1" height="1"/><rect x="10" y="11" width="1" height="1"/></g></svg>`;
export const page = (title, body, status = 200, headers = {}) =>
  new Response(
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="dark"><title>${esc(title)}</title><style>${STYLE}</style>` +
      `<div class="brand">${LOGO}loft <small>push</small></div><main><h1>${esc(title)}</h1>${body}</main></html>`,
    { status, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", ...headers } },
  );
/** True when a setting is missing, or PORTAL_URL isn't an https:// URL with a host (Apple refuses any other redirect address). */
export const incomplete = (env) => REQUIRED.some((k) => !env[k]) || !/^https:\/\/[^/\s]+/i.test(env.PORTAL_URL);
export const notConfigured = (env = {}) =>
  page("Push portal", env.PORTAL_URL && !/^https:\/\/[^/\s]+/i.test(env.PORTAL_URL) ? "<p>The push portal isn't configured: PORTAL_URL must start with https://</p>" : "<p>The push portal isn't configured.</p>", 503);

const hmac = (env, purpose, value) => createHmac("sha256", env.SESSION_SECRET).update(`${purpose}.${value}`).digest("base64url");
/** `value.expiry.signature`, valid until it expires. The purpose ("session" or "state") is signed but not stored, so a cookie sealed for one is refused as the other. */
const seal = (env, purpose, value, seconds, now) => {
  const body = `${Buffer.from(value).toString("base64url")}.${Math.floor(now / 1000) + seconds}`;
  return `${body}.${hmac(env, purpose, body)}`;
};
const unseal = (env, purpose, sealed, now) => {
  const [value, expiry, mac] = (sealed ?? "").split(".");
  if (!value || !expiry || !mac) return null;
  const good = Buffer.from(hmac(env, purpose, `${value}.${expiry}`));
  const given = Buffer.from(mac);
  if (good.length !== given.length || !timingSafeEqual(good, given) || Number(expiry) * 1000 < now) return null;
  return Buffer.from(value, "base64url").toString();
};
export const cookie = (name, value, seconds) => `${name}=${value}; Max-Age=${seconds}; Path=/; HttpOnly; Secure; SameSite=Lax`;
export const cookieOf = (request, name) => (request.headers.get("cookie") ?? "").split(/;\s*/).find((c) => c.startsWith(`${name}=`))?.slice(name.length + 1);
export const redirect = (to, ...cookies) => {
  const headers = new Headers({ location: to });
  cookies.forEach((c) => headers.append("set-cookie", c));
  return new Response(null, { status: 303, headers });
};
export const accountOf = (request, env, now) => unseal(env, "session", cookieOf(request, "session"), now);

/** The secret Apple wants to see: a short ES256 token signed with the sign-in key. */
function clientSecret(env, now) {
  const b64 = (v) => Buffer.from(JSON.stringify(v)).toString("base64url");
  const iat = Math.floor(now / 1000);
  const head = b64({ alg: "ES256", kid: env.APPLE_SIGNIN_KEY_ID });
  const claims = b64({ iss: env.APPLE_TEAM_ID, iat, exp: iat + 300, aud: APPLE, sub: env.APPLE_SERVICES_ID });
  const key = createPrivateKey(env.APPLE_SIGNIN_KEY_P8.replace(/\\n/g, "\n"));
  return `${head}.${claims}.${sign("sha256", Buffer.from(`${head}.${claims}`), { key, dsaEncoding: "ieee-p1363" }).toString("base64url")}`;
}

/**
 * The account's keys, oldest first: their hash (which names a key without being it), name, created and last used.
 * An id whose key is gone (deleted by hand, as the README's runbook says) is dropped from the account's set, so it
 * neither shows nor counts toward the cap.
 */
async function keysOf(store, account) {
  const [ids] = await store.pipeline([["SMEMBERS", `account-keys:${account}`]]);
  if (!ids?.length) return [];
  const got = await store.pipeline(ids.flatMap((id) => [["GET", `relay-key:${id}`], ["GET", `relay-key-used:${id}`]]));
  const gone = ids.filter((_, i) => !got[2 * i]);
  if (gone.length) await store.pipeline([["SREM", `account-keys:${account}`, ...gone]]);
  return ids
    .map((id, i) => got[2 * i] && { id, ...JSON.parse(got[2 * i]), used: got[2 * i + 1] })
    .filter(Boolean)
    .sort((a, b) => a.created.localeCompare(b.created));
}

const day = (iso) => new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" });
/** A last-used date is kept for a year after the last push, so a key older than that without one may have been used before. */
const lastUsed = (k, now) => (k.used ? day(k.used) : Date.parse(k.created) > now - USED_SECONDS * 1000 ? "never" : "not in the last year");
export const capped = () => page("Too many keys", `<p>An account can hold at most ${MAX_KEYS} keys. Revoke one to make room. <a href="/">Back</a></p>`, 409);

/** GET /: the sign-in button, or, signed in, the account's keys and the form to create one. */
export async function home(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = accountOf(request, env, now);
  if (!account) {
    return page("Loft push portal", `<p>Sign in with Apple to get a key for pushing from your Loft.</p><p>${appleButton}</p>`);
  }
  // Signed in on the way to approving a Loft's code: back to that page.
  const pairing = cookieOf(request, "pair");
  if (pairing && /^[A-Za-z0-9_-]{22}$/.test(pairing)) return redirect(`/connect?code=${pairing}`);
  const keys = await keysOf(makeKv(env), account);
  const rows = keys
    .map((k) => `<tr><td>${esc(k.label)}</td><td>${day(k.created)}</td><td>${lastUsed(k, now)}</td><td><form method="post" action="/keys/revoke"><input type="hidden" name="id" value="${k.id}"><button class="danger">Revoke</button></form></td></tr>`)
    .join("");
  const list = keys.length ? `<div class="table"><table><tr><th>Name</th><th>Created</th><th>Last used</th><th></th></tr>${rows}</table></div>` : "<p>No keys yet.</p>";
  const create =
    keys.length < MAX_KEYS
      ? '<form method="post" action="/keys"><p><label>Key name<input name="name" maxlength="60" required placeholder="My Mac"></label></p><button class="primary">Create key</button></form>'
      : "<p>Revoke a key to make room for a new one.</p>";
  return page("Your keys", `${list}<p>${keys.length} of ${MAX_KEYS} keys. A key is shown once, when you create it. Revoke one and its Loft stops pushing at once.</p>${create}<p><a href="/account/delete">Delete my account</a></p>`);
}

/**
 * Sends the user to Apple, with a state that comes back only to this browser. With `deleting` (an account), this
 * sign-in confirms deleting it: the intent is sealed with this state, so it never applies to a later sign-in.
 */
const toApple = (env, now, deleting) => {
  const state = randomBytes(16).toString("base64url");
  const query = new URLSearchParams({ client_id: env.APPLE_SERVICES_ID, redirect_uri: `${origin(env)}/auth/callback`, response_type: "code", state });
  const intent = deleting ? cookie("delete", seal(env, "delete", `${state}.${deleting}`, STATE_SECONDS, now), STATE_SECONDS) : cookie("delete", "", 0);
  return redirect(`${appleHost(env)}/auth/authorize?${query}`, cookie("state", seal(env, "state", state, STATE_SECONDS, now), STATE_SECONDS), intent);
};

/**
 * Counts one `what` ("sign-in", "callback" or "key") from the request's IP this hour: null within PER_HOUR, else a 429
 * page saying when to try again. Vercel sets the first x-forwarded-for address; the store holds only its hash.
 */
export async function limited(store, request, what, now) {
  const ip = (request.headers.get("x-forwarded-for") ?? "").split(",")[0].trim();
  const hour = Math.floor(now / 3600000);
  if ((await store.hit(`limit:${what}:${createHash("sha256").update(ip).digest("hex")}:${hour}`, 3600)) <= PER_HOUR) return null;
  const retry = (hour + 1) * 3600 - Math.floor(now / 1000);
  const tries = what === "key" ? "key creations" : "sign-ins";
  return page("Too many tries", `<p>At most ${PER_HOUR} ${tries} an hour from one network. Try again in ${Math.ceil(retry / 60)} minutes.</p>`, 429, { "retry-after": String(retry) });
}

/** GET /auth/start: Sign in with Apple. */
export async function start(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  return (await limited(makeKv(env), request, "sign-in", now)) ?? toApple(env, now);
}

/** GET /auth/callback: Apple's answer. The account is Apple's user id (`sub`) and nothing else. */
export async function callback(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const limit = await limited(makeKv(env), request, "callback", now);
  if (limit) return limit;
  const params = new URL(request.url).searchParams;
  const state = unseal(env, "state", cookieOf(request, "state"), now);
  const clear = cookie("state", "", 0);
  if (!state || state !== params.get("state") || !params.get("code")) return page("Sign-in failed", '<p>That sign-in didn\'t check out. <a href="/">Try again</a>.</p>', 400, { "set-cookie": clear });
  // `state.account` from `toApple`: only for the sign-in it was made with.
  const intent = unseal(env, "delete", cookieOf(request, "delete"), now) ?? "";
  const deleting = intent.startsWith(`${state}.`) ? intent.slice(state.length + 1) : null;
  let sub, tokens;
  try {
    const res = await fetch(`${appleHost(env)}/auth/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ client_id: env.APPLE_SERVICES_ID, client_secret: clientSecret(env, now), code: params.get("code"), grant_type: "authorization_code", redirect_uri: `${origin(env)}/auth/callback` }),
    });
    if (!res.ok) throw new Error(`Apple answered ${res.status}`);
    // The id token came from Apple over TLS in answer to our own request, so its signature isn't checked again.
    tokens = await res.json();
    const claims = JSON.parse(Buffer.from(tokens.id_token.split(".")[1], "base64url"));
    if (claims.iss !== APPLE || claims.aud !== env.APPLE_SERVICES_ID || claims.exp * 1000 < now || !claims.sub) throw new Error("unexpected id token");
    sub = String(claims.sub);
  } catch (e) {
    console.error(`push portal: sign-in failed (${e.message})`);
    return page("Sign-in failed", '<p>Apple didn\'t confirm the sign-in. <a href="/">Try again</a>.</p>', 502, { "set-cookie": clear });
  }
  if (deleting) return finishDelete(env, now, deleting, sub, tokens);
  await makeKv(env).setIfMissing(`account:${sub}`, JSON.stringify({ created: new Date(now).toISOString() }));
  return redirect("/", clear, cookie("session", seal(env, "session", sub, SESSION_SECONDS, now), SESSION_SECONDS));
}

/** GET /account/delete: what deleting does, and the button. */
export async function confirmDelete(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const n = (await keysOf(makeKv(env), account)).length;
  const what = n === 1 ? "your account and its key, so its Loft stops pushing" : n ? `your account and all ${n} keys, so those Lofts stop pushing` : "your account";
  return page(
    "Delete your account?",
    `<p>This deletes ${what}, and ends Sign in with Apple for this portal. It can't be undone. You can sign in again later and start fresh.</p>` +
      '<p>Apple asks you to sign in once more to confirm.</p><form method="post" action="/account/delete"><div class="row"><button class="danger">Delete my account</button><a href="/">Cancel</a></div></form>',
  );
}

/** POST /account/delete: Apple confirms it's still the account's owner, and hands over the token to revoke (none is stored). */
export async function startDelete(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  return (await limited(makeKv(env), request, "sign-in", now)) ?? toApple(env, now, account);
}

/**
 * Deletes `account`, its keys and its devices' bindings. The account and every key read beforehand go in one transaction, which also reads
 * any key made since; once the account is gone `makeAccountKey` undoes its own key, so those are the last ones.
 * A failure part way leaves the set holding whatever is left, so signing in again shows those keys and deleting
 * again finishes the job.
 */
export async function deleteAccount(store, account) {
  const set = `account-keys:${account}`;
  const records = (ids) => (ids ?? []).flatMap((id) => [`relay-key:${id}`, `relay-key-used:${id}`]);
  const [ids] = await store.pipeline([["SMEMBERS", set]]);
  const drop = ids?.length ? [["SREM", set, ...ids]] : [];
  const answers = await store.transaction([["DEL", `account:${account}`, ...records(ids)], ...drop, ["SMEMBERS", set]]);
  await store.transaction([["DEL", set, ...records(answers.at(-1))]]);
  // Its keys are gone, so its devices are free: each one it still holds (one may have expired and gone to another).
  const devices = `account-devices:${account}`;
  const [hashes] = await store.pipeline([["SMEMBERS", devices]]);
  const owners = hashes?.length ? await store.pipeline(hashes.map((h) => ["GET", `device-owner:${h}`])) : [];
  const held = (hashes ?? []).filter((h, i) => owners[i] === `account:${account}`).map((h) => `device-owner:${h}`);
  await store.transaction([["DEL", devices, ...held]]);
}

/** Back from Apple on the way to deleting: only the account that asked can be deleted. */
async function finishDelete(env, now, account, sub, tokens) {
  const signedOut = [cookie("state", "", 0), cookie("delete", "", 0)];
  if (account !== sub) {
    const res = page("Not deleted", '<p>That was a different Apple ID from this account\'s, so nothing was deleted. <a href="/">Back</a></p>', 403);
    signedOut.forEach((c) => res.headers.append("set-cookie", c));
    return res;
  }
  try {
    await deleteAccount(makeKv(env), account);
  } catch (e) {
    console.error(`push portal: deleting an account failed (${e.message})`);
    const res = page("Not deleted yet", '<p>Your account couldn\'t be deleted completely. <a href="/account/delete">Try again</a>.</p>', 502);
    signedOut.forEach((c) => res.headers.append("set-cookie", c));
    return res;
  }
  const revoked = await revokeApple(env, tokens, now);
  const res = page(
    "Account deleted",
    `<p>Your account and its keys are gone, and Lofts using them no longer push.${revoked ? "" : " Apple didn't confirm ending the sign-in: you can remove this portal under Sign in with Apple in your Apple ID settings."}</p>`,
  );
  [...signedOut, cookie("session", "", 0)].forEach((c) => res.headers.append("set-cookie", c));
  return res;
}

/** Ends the Sign in with Apple link for this portal by revoking the token Apple just gave. True when Apple accepted it. */
async function revokeApple(env, tokens, now) {
  const [token, hint] = tokens.refresh_token ? [tokens.refresh_token, "refresh_token"] : [tokens.access_token, "access_token"];
  if (!token) return false;
  try {
    const res = await fetch(`${appleHost(env)}/auth/revoke`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ client_id: env.APPLE_SERVICES_ID, client_secret: clientSecret(env, now), token, token_type_hint: hint }),
    });
    if (!res.ok) throw new Error(`Apple answered ${res.status}`);
    return true;
  } catch (e) {
    console.error(`push portal: token revocation failed (${e.message})`);
    return false;
  }
}

/**
 * Creates a key for `account`: `{ key }`, or `{ error }` when the account is gone or already holds MAX_KEYS.
 * Written and counted in one transaction, then undone if it went over: two creates at once can't both make the 5th.
 */
export async function makeAccountKey(store, account, name, now) {
  await keysOf(store, account); // drops ids whose key is gone, so they don't count
  const { key, id, record } = newKey(name, now, account);
  const [exists, , , count] = await store.transaction([["EXISTS", `account:${account}`], ["SET", `relay-key:${id}`, record], ["SADD", `account-keys:${account}`, id], ["SCARD", `account-keys:${account}`]]);
  if (exists !== 1 || count > MAX_KEYS) await store.transaction([["DEL", `relay-key:${id}`], ["SREM", `account-keys:${account}`, id]]);
  if (exists !== 1) return { error: "gone" };
  if (count > MAX_KEYS) return { error: "capped" };
  return { key };
}

/** POST /keys with `name`: creates a key for the signed-in account and shows it, once. */
export async function createAccountKey(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const name = String((await request.formData()).get("name") ?? "").trim().slice(0, 60);
  if (!name) return page("Name the key", '<p>A key needs a name. <a href="/">Back</a></p>', 400);
  const store = makeKv(env);
  const limit = await limited(store, request, "key", now);
  if (limit) return limit;
  const made = await makeAccountKey(store, account, name, now);
  // A session can outlive its account (deleted since): it's signed out rather than given a key nobody owns.
  if (made.error === "gone") return redirect("/", cookie("session", "", 0));
  if (made.error === "capped") return capped();
  const { key } = made;
  return page("Your new key", `<p>Copy it now. It is shown only this once.</p><code>${esc(key)}</code><p>Paste it in Loft → Settings → Push notifications.</p>`);
}

/** POST /keys/revoke with `id` (the key's hash): deletes one of the signed-in account's keys. The relay refuses it from the next push. */
export async function revokeAccountKey(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const id = String((await request.formData()).get("id") ?? "");
  const store = makeKv(env);
  const [mine] = /^[0-9a-f]{64}$/.test(id) ? await store.pipeline([["SISMEMBER", `account-keys:${account}`, id]]) : [0];
  if (mine !== 1) return page("No such key", '<p>That key isn\'t one of yours, or it\'s already revoked. <a href="/">Back</a></p>', 404);
  await store.transaction([["DEL", `relay-key:${id}`, `relay-key-used:${id}`], ["SREM", `account-keys:${account}`, id]]);
  return redirect("/");
}
