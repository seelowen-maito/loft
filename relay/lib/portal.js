// The push portal (ADR 0003): a web page where a user signs in with Apple and creates a relay key. It stores only
// Apple's stable user id and hashed keys; no email or name is asked for. The functions in api/ call one handler each.
// Every Apple value comes from the environment (a self-hoster sets their own); with any missing the portal says so.

import { createHmac, createPrivateKey, randomBytes, sign, timingSafeEqual } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { newKey, MAX_KEYS, USED_SECONDS } from "./relay.js";

export const REQUIRED = ["APPLE_SERVICES_ID", "APPLE_TEAM_ID", "APPLE_SIGNIN_KEY_ID", "APPLE_SIGNIN_KEY_P8", "PORTAL_URL", "SESSION_SECRET", "KV_REST_API_URL", "KV_REST_API_TOKEN"];
const APPLE = "https://appleid.apple.com";
const SESSION_SECONDS = 7 * 86400;
const STATE_SECONDS = 600;

/** Apple's sign-in host; APPLE_ID_HOST replaces it for the tests' fake Apple, never in production. */
const appleHost = (env) => (env.VERCEL_ENV !== "production" && env.APPLE_ID_HOST) || APPLE;
const origin = (env) => env.PORTAL_URL.replace(/\/+$/, "");

export const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
export const page = (title, body, status = 200, headers = {}) =>
  new Response(
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${esc(title)}</title>` +
      `<style>body{font:16px/1.5 system-ui,sans-serif;max-width:32rem;margin:3rem auto;padding:0 1rem}button,input{font:inherit;padding:.5rem .75rem}code{word-break:break-all;display:block;padding:.75rem;background:#eee;border-radius:6px}table{width:100%;border-collapse:collapse;margin:1rem 0}th,td{text-align:left;padding:.4rem .25rem;border-bottom:1px solid #ddd}td form{margin:0}</style>` +
      `<h1>${esc(title)}</h1>${body}</html>`,
    { status, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", ...headers } },
  );
export const notConfigured = () => page("Push portal", "<p>The push portal isn't configured.</p>", 503);

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
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const account = accountOf(request, env, now);
  if (!account) {
    return page("Loft push portal", '<p>Sign in with Apple to get a key for pushing from your Loft.</p><p><a href="/auth/start"><button>Sign in with Apple</button></a></p>');
  }
  // Signed in on the way to approving a Loft's code: back to that page.
  const pairing = cookieOf(request, "pair");
  if (pairing && /^[A-Za-z0-9_-]{22}$/.test(pairing)) return redirect(`/connect?code=${pairing}`);
  const keys = await keysOf(makeKv(env), account);
  const rows = keys
    .map((k) => `<tr><td>${esc(k.label)}</td><td>${day(k.created)}</td><td>${lastUsed(k, now)}</td><td><form method="post" action="/keys/revoke"><input type="hidden" name="id" value="${k.id}"><button>Revoke</button></form></td></tr>`)
    .join("");
  const list = keys.length ? `<table><tr><th>Name</th><th>Created</th><th>Last used</th><th></th></tr>${rows}</table>` : "<p>No keys yet.</p>";
  const create =
    keys.length < MAX_KEYS
      ? '<form method="post" action="/keys"><p><label>Key name <input name="name" maxlength="60" required placeholder="My Mac"></label></p><button>Create key</button></form>'
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

/** GET /auth/start: Sign in with Apple. */
export async function start(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  return toApple(env, now);
}

/** GET /auth/callback: Apple's answer. The account is Apple's user id (`sub`) and nothing else. */
export async function callback(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
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
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const n = (await keysOf(makeKv(env), account)).length;
  const keys = n === 1 ? "your key, so its Loft stops" : n ? `all ${n} keys, so those Lofts stop` : "your account, so new Lofts can't";
  return page(
    "Delete your account?",
    `<p>This deletes ${keys} pushing, and ends Sign in with Apple for this portal. It can't be undone. You can sign in again later and start fresh.</p>` +
      '<p>Apple asks you to sign in once more to confirm.</p><form method="post" action="/account/delete"><button>Delete my account</button> <a href="/">Cancel</a></form>',
  );
}

/** POST /account/delete: Apple confirms it's still the account's owner, and hands over the token to revoke (none is stored). */
export async function startDelete(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  return toApple(env, now, account);
}

/**
 * Deletes `account` and its keys. The account goes first: from then on no key can be made for it, so every key it
 * has is in its set when the set is read.
 */
export async function deleteAccount(store, account) {
  await store.transaction([["DEL", `account:${account}`]]);
  const [ids] = await store.pipeline([["SMEMBERS", `account-keys:${account}`]]);
  await store.transaction([["DEL", `account-keys:${account}`, ...(ids ?? []).flatMap((id) => [`relay-key:${id}`, `relay-key-used:${id}`])]]);
}

/** Back from Apple on the way to deleting: only the account that asked can be deleted. */
async function finishDelete(env, now, account, sub, tokens) {
  const signedOut = [cookie("state", "", 0), cookie("delete", "", 0)];
  if (account !== sub) {
    const res = page("Not deleted", '<p>That was a different Apple ID from this account\'s, so nothing was deleted. <a href="/">Back</a></p>', 403);
    signedOut.forEach((c) => res.headers.append("set-cookie", c));
    return res;
  }
  await deleteAccount(makeKv(env), account);
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
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const name = String((await request.formData()).get("name") ?? "").trim().slice(0, 60);
  if (!name) return page("Name the key", '<p>A key needs a name. <a href="/">Back</a></p>', 400);
  const made = await makeAccountKey(makeKv(env), account, name, now);
  // A session can outlive its account (deleted since): it's signed out rather than given a key nobody owns.
  if (made.error === "gone") return redirect("/", cookie("session", "", 0));
  if (made.error === "capped") return capped();
  const { key } = made;
  return page("Your new key", `<p>Copy it now. It is shown only this once.</p><code>${esc(key)}</code><p>Paste it in Loft → Settings → Push notifications.</p>`);
}

/** POST /keys/revoke with `id` (the key's hash): deletes one of the signed-in account's keys. The relay refuses it from the next push. */
export async function revokeAccountKey(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const id = String((await request.formData()).get("id") ?? "");
  const store = makeKv(env);
  const [mine] = /^[0-9a-f]{64}$/.test(id) ? await store.pipeline([["SISMEMBER", `account-keys:${account}`, id]]) : [0];
  if (mine !== 1) return page("No such key", '<p>That key isn\'t one of yours, or it\'s already revoked. <a href="/">Back</a></p>', 404);
  await store.transaction([["DEL", `relay-key:${id}`, `relay-key-used:${id}`], ["SREM", `account-keys:${account}`, id]]);
  return redirect("/");
}
