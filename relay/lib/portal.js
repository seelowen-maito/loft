// The push portal (ADR 0003): a web page where a user signs in with Apple and creates a relay key. It stores only
// Apple's stable user id and hashed keys; no email or name is asked for. The functions in api/ call one handler each.
// Every Apple value comes from the environment (a self-hoster sets their own); with any missing the portal says so.

import { createHmac, createPrivateKey, randomBytes, sign, timingSafeEqual } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { createKey } from "./relay.js";

const REQUIRED = ["APPLE_SERVICES_ID", "APPLE_TEAM_ID", "APPLE_SIGNIN_KEY_ID", "APPLE_SIGNIN_KEY_P8", "PORTAL_URL", "SESSION_SECRET", "KV_REST_API_URL", "KV_REST_API_TOKEN"];
const APPLE = "https://appleid.apple.com";
const SESSION_SECONDS = 7 * 86400;
const STATE_SECONDS = 600;

/** Apple's sign-in host; APPLE_ID_HOST replaces it for the tests' fake Apple, never in production. */
const appleHost = (env) => (env.VERCEL_ENV !== "production" && env.APPLE_ID_HOST) || APPLE;
const origin = (env) => env.PORTAL_URL.replace(/\/+$/, "");

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const page = (title, body, status = 200, headers = {}) =>
  new Response(
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${esc(title)}</title>` +
      `<style>body{font:16px/1.5 system-ui,sans-serif;max-width:32rem;margin:3rem auto;padding:0 1rem}button,input{font:inherit;padding:.5rem .75rem}code{word-break:break-all;display:block;padding:.75rem;background:#eee;border-radius:6px}</style>` +
      `<h1>${esc(title)}</h1>${body}</html>`,
    { status, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", ...headers } },
  );
const notConfigured = () => page("Push portal", "<p>The push portal isn't configured.</p>", 503);

const hmac = (env, value) => createHmac("sha256", env.SESSION_SECRET).update(value).digest("base64url");
/** `value.expiry.signature`, valid until it expires. Used for the session and the sign-in state. */
const seal = (env, value, seconds, now) => {
  const body = `${Buffer.from(value).toString("base64url")}.${Math.floor(now / 1000) + seconds}`;
  return `${body}.${hmac(env, body)}`;
};
const unseal = (env, sealed, now) => {
  const [value, expiry, mac] = (sealed ?? "").split(".");
  if (!value || !expiry || !mac) return null;
  const good = Buffer.from(hmac(env, `${value}.${expiry}`));
  const given = Buffer.from(mac);
  if (good.length !== given.length || !timingSafeEqual(good, given) || Number(expiry) * 1000 < now) return null;
  return Buffer.from(value, "base64url").toString();
};
const cookie = (name, value, seconds) => `${name}=${value}; Max-Age=${seconds}; Path=/; HttpOnly; Secure; SameSite=Lax`;
const cookieOf = (request, name) => (request.headers.get("cookie") ?? "").split(/;\s*/).find((c) => c.startsWith(`${name}=`))?.slice(name.length + 1);
const redirect = (to, ...cookies) => {
  const headers = new Headers({ location: to });
  cookies.forEach((c) => headers.append("set-cookie", c));
  return new Response(null, { status: 303, headers });
};
const accountOf = (request, env, now) => unseal(env, cookieOf(request, "session"), now);

/** The secret Apple wants to see: a short ES256 token signed with the sign-in key. */
function clientSecret(env, now) {
  const b64 = (v) => Buffer.from(JSON.stringify(v)).toString("base64url");
  const iat = Math.floor(now / 1000);
  const head = b64({ alg: "ES256", kid: env.APPLE_SIGNIN_KEY_ID });
  const claims = b64({ iss: env.APPLE_TEAM_ID, iat, exp: iat + 300, aud: APPLE, sub: env.APPLE_SERVICES_ID });
  const key = createPrivateKey(env.APPLE_SIGNIN_KEY_P8.replace(/\\n/g, "\n"));
  return `${head}.${claims}.${sign("sha256", Buffer.from(`${head}.${claims}`), { key, dsaEncoding: "ieee-p1363" }).toString("base64url")}`;
}

/** GET /: the sign-in button, or, signed in, the form to create a key. */
export async function home(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  if (!accountOf(request, env, now)) {
    return page("Loft push portal", '<p>Sign in with Apple to get a key for pushing from your Loft.</p><p><a href="/auth/start"><button>Sign in with Apple</button></a></p>');
  }
  return page("Loft push portal", '<form method="post" action="/keys"><p><label>Key name <input name="name" maxlength="60" required placeholder="My Mac"></label></p><button>Create key</button></form>');
}

/** GET /auth/start: sends the user to Apple, with a state that comes back only to this browser. */
export async function start(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const state = randomBytes(16).toString("base64url");
  const query = new URLSearchParams({ client_id: env.APPLE_SERVICES_ID, redirect_uri: `${origin(env)}/auth/callback`, response_type: "code", state });
  return redirect(`${appleHost(env)}/auth/authorize?${query}`, cookie("state", seal(env, state, STATE_SECONDS, now), STATE_SECONDS));
}

/** GET /auth/callback: Apple's answer. The account is Apple's user id (`sub`) and nothing else. */
export async function callback(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const params = new URL(request.url).searchParams;
  const state = unseal(env, cookieOf(request, "state"), now);
  const clear = cookie("state", "", 0);
  if (!state || state !== params.get("state") || !params.get("code")) return page("Sign-in failed", '<p>That sign-in didn\'t check out. <a href="/">Try again</a>.</p>', 400, { "set-cookie": clear });
  let sub;
  try {
    const res = await fetch(`${appleHost(env)}/auth/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ client_id: env.APPLE_SERVICES_ID, client_secret: clientSecret(env, now), code: params.get("code"), grant_type: "authorization_code", redirect_uri: `${origin(env)}/auth/callback` }),
    });
    if (!res.ok) throw new Error(`Apple answered ${res.status}`);
    // The id token came from Apple over TLS in answer to our own request, so its signature isn't checked again.
    const claims = JSON.parse(Buffer.from((await res.json()).id_token.split(".")[1], "base64url"));
    if (claims.iss !== APPLE || claims.aud !== env.APPLE_SERVICES_ID || claims.exp * 1000 < now || !claims.sub) throw new Error("unexpected id token");
    sub = String(claims.sub);
  } catch (e) {
    console.error(`push portal: sign-in failed (${e.message})`);
    return page("Sign-in failed", '<p>Apple didn\'t confirm the sign-in. <a href="/">Try again</a>.</p>', 502, { "set-cookie": clear });
  }
  await makeKv(env).setIfMissing(`account:${sub}`, JSON.stringify({ created: new Date(now).toISOString() }));
  return redirect("/", clear, cookie("session", seal(env, sub, SESSION_SECONDS, now), SESSION_SECONDS));
}

/** POST /keys with `name`: creates a key for the signed-in account and shows it, once. */
export async function createAccountKey(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const name = String((await request.formData()).get("name") ?? "").trim().slice(0, 60);
  if (!name) return page("Name the key", '<p>A key needs a name. <a href="/">Back</a></p>', 400);
  const key = await createKey(env, name, now, account);
  return page("Your new key", `<p>Copy it now. It is shown only this once.</p><code>${esc(key)}</code><p>Paste it in Loft → Settings → Push notifications.</p>`);
}
