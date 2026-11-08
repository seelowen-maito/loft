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
  `*{box-sizing:border-box}body{margin:0;min-height:100vh;background:var(--ink-950);color:var(--fg);font:15px/1.55 Geist,ui-sans-serif,system-ui,sans-serif;-webkit-font-smoothing:antialiased}` +
  `::selection{background:rgb(200 242 109/.3)}header{display:flex;align-items:center;justify-content:space-between;gap:1rem;padding:.8rem clamp(1rem,3vw,2rem);border-bottom:1px solid var(--line)}a.brand{display:flex;align-items:center;gap:.6rem;font:500 13.5px "Geist Mono",ui-monospace,monospace;text-decoration:none}.brand .mark{border-radius:8px}` +
  `.who{display:flex;align-items:center;gap:.7rem;color:var(--fg-3);font-size:13px}.who svg{height:14px;width:auto;fill:var(--fg-3)}.who form{margin:0}.who span{white-space:nowrap}` +
  `main{width:100%;max-width:44rem;margin:0 auto;padding:clamp(1.5rem,6vh,3rem) 1rem 2rem}main.center{max-width:24rem;text-align:center;padding-top:clamp(2rem,14vh,7rem)}main.card{width:calc(100% - 2rem);max-width:32rem;margin-top:clamp(1rem,8vh,4rem);padding:1.5rem;background:var(--ink-900);border:1px solid var(--line-strong);border-radius:1rem}` +
  `.title{display:flex;align-items:flex-end;justify-content:space-between;gap:1rem;margin-bottom:1rem}.title small{display:block;font:400 10.5px "Geist Mono",ui-monospace,monospace;text-transform:uppercase;letter-spacing:.14em;color:var(--fg-3)}h1{margin:0;font-size:22px;line-height:1.3;font-weight:500;letter-spacing:-.015em}.card h1{font-size:16px}.center .title{justify-content:center}.note{font-size:12px;color:var(--fg-3)}p{margin:0 0 1rem;color:var(--fg-2)}main>:last-child{margin-bottom:0}` +
  `a{color:var(--fg);text-decoration-color:var(--line-strong);text-underline-offset:3px}a:hover{text-decoration-color:var(--accent)}form{margin:0 0 1rem}label{display:block;color:var(--fg-2);font-size:13px}` +
  `input:not([type=hidden]){display:block;width:100%;margin-top:.4rem;padding:.55rem .75rem;font:inherit;font-size:14px;color:var(--fg);background:var(--ink-850);border:1px solid var(--line-strong);border-radius:.75rem;outline:0}input::placeholder{color:var(--fg-3)}input:focus{border-color:rgb(200 242 109/.5);box-shadow:0 0 0 3px rgb(200 242 109/.1)}` +
  `button{font:inherit;font-size:13px;padding:.45rem .95rem;color:var(--fg);background:var(--ink-800);border:1px solid var(--line-strong);border-radius:999px;cursor:pointer}button:hover{background:var(--ink-700);border-color:rgb(255 255 255/.2)}button.primary{color:var(--ink-950);font-weight:500;background:var(--accent);border-color:var(--accent)}button.primary:hover{filter:brightness(1.1)}button.danger{color:var(--bad);border-color:rgb(242 125 109/.4);background:transparent}button.danger:hover{background:rgb(242 125 109/.1)}` +
  `.apple{display:inline-flex;align-items:center;justify-content:center;gap:.5rem;min-width:15rem;height:44px;padding:0 1.25rem;color:#000;background:#fff;border:1px solid #fff;border-radius:8px;font-size:17px;font-weight:500;text-decoration:none;white-space:nowrap}.apple:hover{background:#eee}.apple svg{height:20px;width:auto;fill:#000}` +
  `code{display:block;word-break:break-all;margin:0 0 1rem;padding:.8rem .9rem;font:13px/1.5 "Geist Mono",ui-monospace,"SF Mono",monospace;color:var(--accent);background:var(--ink-850);border:1px solid var(--line-strong);border-radius:.75rem;user-select:all}` +
  `.table{overflow-x:auto;margin:0 0 1rem;background:var(--ink-900);border:1px solid var(--line);border-radius:1rem}table{width:100%;border-collapse:collapse;font-size:14px}th{padding:.7rem 1rem;text-align:left;font:400 10.5px "Geist Mono",ui-monospace,monospace;text-transform:uppercase;letter-spacing:.14em;color:var(--fg-3);white-space:nowrap}td{padding:.95rem 1rem;text-align:left;border-top:1px solid var(--line);white-space:nowrap}td.mono{font:13px "Geist Mono",ui-monospace,monospace;color:var(--fg-3)}td:last-child,th:last-child{text-align:right}td form{margin:0}svg.i{height:15px;width:15px;margin-right:.55rem;vertical-align:-2px;fill:none;stroke:var(--fg-3);stroke-width:1.5;stroke-linecap:round;stroke-linejoin:round}` +
  `button.ghost{display:inline-flex;align-items:center;gap:.35rem;padding:.2rem .4rem;color:var(--fg-3);background:none;border:0}button.ghost:hover{color:var(--bad);background:none}button.ghost svg.i{margin:0;stroke:currentColor}` +
  `a.pill{display:inline-block;padding:.45rem .95rem;font-size:13px;color:var(--fg);border:1px solid var(--line-strong);border-radius:999px;text-decoration:none}a.pill.danger{color:var(--bad);border-color:rgb(242 125 109/.4)}a.pill.danger:hover{background:rgb(242 125 109/.1)}a.pill.solid{color:var(--ink-950);background:var(--bad);border-color:var(--bad)}a.pill.primary{color:var(--ink-950);font-weight:500;background:var(--accent);border-color:var(--accent)}.new{display:none}.new:target{display:block}a.btn{text-decoration:none}.btn{display:inline-flex;align-items:center;gap:.4rem;padding:.45rem .95rem;font-size:13px;border-radius:999px;font-weight:500;color:var(--ink-950);background:var(--accent)}.btn:hover{filter:brightness(1.1)}` +
  `.new{margin:0 0 1rem;padding:1rem;background:var(--ink-900);border:1px solid var(--line);border-radius:1rem}.new p{margin:0 0 .8rem}.foot{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:.9rem;margin-top:1rem;padding-top:1rem;border-top:1px solid var(--line);font-size:13px;color:var(--fg-3)}.foot p{margin:0;color:var(--fg-3)}` +
  `.center .apple{width:100%}.card .row{justify-content:flex-end;margin-top:1rem}button.danger.solid{color:var(--ink-950);background:var(--bad);border-color:var(--bad)}button.danger.solid:hover{filter:brightness(1.1)}.warn{color:#e8b64c}` +
  `.row{display:flex;flex-wrap:wrap;align-items:center;gap:.9rem}@media(max-width:520px){.who span{display:none}}`;
const APPLE_LOGO = '<svg viewBox="0 0 814 1000" aria-hidden="true"><path d="M788.1 340.9c-5.8 4.5-108.2 62.2-108.2 190.5 0 148.4 130.3 200.9 134.2 202.2-.6 3.2-20.7 71.9-68.7 141.9-42.8 61.6-87.5 123.1-155.5 123.1s-85.5-39.5-164-39.5c-76.5 0-103.7 40.8-165.9 40.8s-105.6-57-155.5-127C46.7 790.7 0 663 0 541.8c0-194.4 126.4-297.5 250.8-297.5 66.1 0 121.2 43.4 162.7 43.4 39.5 0 101.1-46 176.3-46 28.5 0 130.9 2.6 198.3 99.2zm-234-181.5c31.1-36.9 53.1-88.1 53.1-139.3 0-7.1-.6-14.3-1.9-20.1-50.6 1.9-110.8 33.7-147.1 75.8-28.5 32.4-55.1 83.6-55.1 135.5 0 7.8 1.3 15.6 1.9 18.1 3.2.6 8.4 1.3 13.6 1.3 45.4 0 102.5-30.4 135.5-71.3z"/></svg>';
/** Apple's white "Sign in with Apple" button (their guidelines: white on dark, logo left of the label). */
export const appleButton = '<a class="apple" href="/auth/start">' + APPLE_LOGO + "Sign in with Apple</a>";
const LOGO = `<svg class="mark" width="28" height="28" aria-hidden="true" viewBox="0 0 16 16" shape-rendering="crispEdges"><rect width="16" height="16" rx="3.5" fill="#08090a"/><g fill="#c8f26d"><rect x="7" y="4" width="1" height="1"/><rect x="8" y="4" width="1" height="1"/><rect x="6" y="5" width="1" height="1"/><rect x="7" y="5" width="1" height="1"/><rect x="8" y="5" width="1" height="1"/><rect x="9" y="5" width="1" height="1"/><rect x="5" y="6" width="1" height="1"/><rect x="6" y="6" width="1" height="1"/><rect x="7" y="6" width="1" height="1"/><rect x="8" y="6" width="1" height="1"/><rect x="9" y="6" width="1" height="1"/><rect x="10" y="6" width="1" height="1"/><rect x="4" y="7" width="1" height="1"/><rect x="5" y="7" width="1" height="1"/><rect x="6" y="7" width="1" height="1"/><rect x="7" y="7" width="1" height="1"/><rect x="8" y="7" width="1" height="1"/><rect x="9" y="7" width="1" height="1"/><rect x="10" y="7" width="1" height="1"/><rect x="11" y="7" width="1" height="1"/><rect x="5" y="8" width="1" height="1"/><rect x="10" y="8" width="1" height="1"/><rect x="5" y="9" width="1" height="1"/><rect x="7" y="9" width="1" height="1"/><rect x="8" y="9" width="1" height="1"/><rect x="10" y="9" width="1" height="1"/><rect x="5" y="10" width="1" height="1"/><rect x="7" y="10" width="1" height="1"/><rect x="8" y="10" width="1" height="1"/><rect x="10" y="10" width="1" height="1"/><rect x="5" y="11" width="1" height="1"/><rect x="6" y="11" width="1" height="1"/><rect x="7" y="11" width="1" height="1"/><rect x="8" y="11" width="1" height="1"/><rect x="9" y="11" width="1" height="1"/><rect x="10" y="11" width="1" height="1"/></g></svg>`;
// No scripts at all. The inline <style> needs 'unsafe-inline'; form-action names Apple because a form's redirect counts too (Delete my account ends at Apple).
const CSP = `default-src 'none'; style-src 'unsafe-inline'; font-src 'self'; img-src 'self'; form-action 'self' ${APPLE}; base-uri 'none'; frame-ancestors 'none'`;
const KEY_ICON = '<svg class="i" viewBox="0 0 16 16" aria-hidden="true"><circle cx="5.5" cy="10.5" r="2.8"/><path d="M7.5 8.5 13 3m-2 2 2 2"/></svg>';
const TRASH_ICON = '<svg class="i" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8h5.8l.6-8"/></svg>';
/**
 * A portal page. `opts`: `signedIn` adds the header's "Signed in with Apple" and Sign out; `eyebrow` and `action` sit
 * around the title; `layout` is "center" (the sign-in page) or "card" (a one-question page).
 */
export const page = (title, body, status = 200, headers = {}, opts = {}) =>
  new Response(
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="dark"><title>${esc(title)}</title><style>${STYLE}</style>` +
      `<header><a class="brand" href="/">${LOGO}Loft push portal</a>` +
      (opts.signedIn ? `<div class="who">${APPLE_LOGO}<span>Signed in with Apple</span><form method="post" action="/signout"><button>Sign out</button></form></div>` : "") +
      `</header><main${opts.layout ? ` class="${opts.layout}"` : ""}><div class="title"><div>${opts.eyebrow ? `<small>${esc(opts.eyebrow)}</small>` : ""}<h1>${esc(title)}</h1></div>${opts.action ?? ""}</div>${body}</main></html>`,
    { status, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", "content-security-policy": CSP, "referrer-policy": "no-referrer", "x-content-type-options": "nosniff", ...headers } },
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
// `__Host-` cookies must be Secure, Path=/ and have no Domain, so no sibling subdomain can set or shadow them.
export const cookie = (name, value, seconds) => `__Host-${name}=${value}; Max-Age=${seconds}; Path=/; HttpOnly; Secure; SameSite=Lax`;
export const cookieOf = (request, name) => (request.headers.get("cookie") ?? "").split(/;\s*/).find((c) => c.startsWith(`__Host-${name}=`))?.slice(name.length + 8);
/** A 403 page unless this POST came from the portal itself: `Sec-Fetch-Site: same-origin`, or with no such header an `Origin` equal to PORTAL_URL's. Else null. */
export const crossSite = (request, env) => {
  const site = request.headers.get("sec-fetch-site");
  if (site ? site === "same-origin" : request.headers.get("origin") === new URL(origin(env)).origin) return null;
  return page("Refused", "<p>That request didn't come from this portal. <a href=\"/\">Back</a></p>", 403);
};
export const redirect = (to, ...cookies) => {
  const headers = new Headers({ location: to });
  cookies.forEach((c) => headers.append("set-cookie", c));
  return new Response(null, { status: 303, headers });
};
/** The store key that marks one session cookie as signed out (by its hash; the cookie itself is never stored). */
const sessionEnded = (cookieValue) => `session-ended:${createHash("sha256").update(cookieValue).digest("hex")}`;
/**
 * The signed-in account, or null. The session holds `created|sub`: it counts only while the stored account exists with
 * that created stamp, so deleting the account ends every session, even when the same Apple ID signs up again.
 */
export async function accountOf(request, env, now) {
  const cookieValue = cookieOf(request, "session");
  const [created, ...rest] = (unseal(env, "session", cookieValue, now) ?? "").split("|");
  const sub = rest.join("|");
  if (!created || !sub) return null;
  const [stored, ended] = await makeKv(env).pipeline([["GET", `account:${sub}`], ["EXISTS", sessionEnded(cookieValue)]]);
  try {
    return stored && !ended && JSON.parse(stored).created === created ? sub : null;
  } catch {
    return null;
  }
}
/** Back to the sign-in page; a session cookie that no longer counts is cleared on the way. */
export const endSession = (request) => (cookieOf(request, "session") ? redirect("/", cookie("session", "", 0)) : redirect("/"));

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

const SIGNED_IN = { signedIn: true };
const day = (iso) => new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" });
/** A last-used date is kept for a year after the last push, so a key older than that without one may have been used before. */
const lastUsed = (k, now) => (k.used ? day(k.used) : Date.parse(k.created) > now - USED_SECONDS * 1000 ? "never" : "not in the last year");
export const capped = () => page("Too many keys", `<p>An account can hold at most ${MAX_KEYS} keys. Revoke one to make room. <a href="/">Back</a></p>`, 409, {}, SIGNED_IN);

/** GET /: the sign-in button, or, signed in, the account's keys and the form to create one. */
export async function home(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = await accountOf(request, env, now);
  if (!account) {
    return page("Get push for your Loft", `<p>Sign in to create a relay key. The relay sends your Loft's alerts to your iPhone, with no Apple developer account of your own.</p><p>${appleButton}</p><p class="note">We keep only Apple's anonymous user id and a hash of each key. No email, no name, no alert text.</p>`, 200, cookieOf(request, "session") ? { "set-cookie": cookie("session", "", 0) } : {}, { layout: "center" });
  }
  // Signed in on the way to approving a Loft's code: back to that page.
  const pairing = cookieOf(request, "pair");
  if (pairing && /^[A-Za-z0-9_-]{22}$/.test(pairing)) return redirect(`/connect?code=${pairing}`);
  const keys = await keysOf(makeKv(env), account);
  const rows = keys
    .map((k) => `<tr><td>${KEY_ICON}${esc(k.label)}</td><td class="mono">${day(k.created)}</td><td class="mono">${lastUsed(k, now)}</td><td><form method="post" action="/keys/revoke"><input type="hidden" name="id" value="${k.id}"><button class="ghost">${TRASH_ICON}Revoke</button></form></td></tr>`)
    .join("");
  const list = keys.length ? `<div class="table"><table><tr><th>Name</th><th>Created</th><th>Last used</th><th></th></tr>${rows}</table></div>` : "<p>No keys yet.</p>";
  // The name form opens through the link's #new anchor (CSS :target), so no script is needed.
  const room = keys.length < MAX_KEYS;
  const create = room ? '<form id="new" class="new" method="post" action="/keys"><p><label>Key name<input name="name" maxlength="60" required placeholder="My Mac"></label></p><button class="primary">Create key</button></form>' : "";
  return page(
    "Your keys",
    `${create}${list}<p class="note">${keys.length} of ${MAX_KEYS} keys. A key is shown once, when you create it. Revoke one and its Loft stops pushing within a minute.</p>` +
      '<div class="foot"><p>Delete my account removes your keys and Apple\'s sign-in link.</p><a class="pill danger" href="/account/delete">Delete my account</a></div>',
    200,
    {},
    { ...SIGNED_IN, eyebrow: "Relay keys", action: room ? '<a class="btn" href="#new">+ New key</a>' : "" },
  );
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
  const store = makeKv(env);
  await store.setIfMissing(`account:${sub}`, JSON.stringify({ created: new Date(now).toISOString() }));
  const { created } = JSON.parse(await store.get(`account:${sub}`));
  return redirect("/", clear, cookie("session", seal(env, "session", `${created}|${sub}`, SESSION_SECONDS, now), SESSION_SECONDS));
}

/** POST /signout: ends this browser's session, so its old cookie no longer counts even if someone kept a copy. */
export async function signOut(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const refused = crossSite(request, env);
  if (refused) return refused;
  const cookieValue = cookieOf(request, "session");
  // Only a genuine session is recorded, for as long as it has left; a forged cookie costs no store command.
  if (unseal(env, "session", cookieValue, now)) await makeKv(env).setEx(sessionEnded(cookieValue), "1", Number(cookieValue.split(".")[1]) - Math.floor(now / 1000));
  return redirect("/", cookie("session", "", 0));
}

/** GET /account/delete: what deleting does, and the button. */
export async function confirmDelete(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = await accountOf(request, env, now);
  if (!account) return endSession(request);
  const n = (await keysOf(makeKv(env), account)).length;
  const what = n === 1 ? "your account and its key, so its Loft stops pushing" : n ? `your account and all <b>${n} keys</b>, so those Lofts stop pushing` : "your account";
  return page(
    "Delete your account?",
    `<p>This deletes ${what}, and ends Sign in with Apple for this portal. It can't be undone. You can sign in again later and start fresh.</p>` +
      '<p>Apple asks you to sign in once more to confirm.</p><form method="post" action="/account/delete"><div class="row"><a class="pill" href="/">Cancel</a><button class="danger solid">Delete my account</button></div></form>',
    200,
    {},
    { ...SIGNED_IN, layout: "card" },
  );
}

/** POST /account/delete: Apple confirms it's still the account's owner, and hands over the token to revoke (none is stored). */
export async function startDelete(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const refused = crossSite(request, env);
  if (refused) return refused;
  const account = await accountOf(request, env, now);
  if (!account) return endSession(request);
  return (await limited(makeKv(env), request, "sign-in", now)) ?? toApple(env, now, account);
}

/**
 * Deletes `account` and its keys. The account and every key read beforehand go in one transaction, which also reads
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
  const refused = crossSite(request, env);
  if (refused) return refused;
  const account = await accountOf(request, env, now);
  if (!account) return endSession(request);
  const name = String((await request.formData()).get("name") ?? "").trim().slice(0, 60);
  if (!name) return page("Name the key", '<p>A key needs a name. <a href="/">Back</a></p>', 400, {}, SIGNED_IN);
  const store = makeKv(env);
  const limit = await limited(store, request, "key", now);
  if (limit) return limit;
  const made = await makeAccountKey(store, account, name, now);
  // A session can outlive its account (deleted since): it's signed out rather than given a key nobody owns.
  if (made.error === "gone") return redirect("/", cookie("session", "", 0));
  if (made.error === "capped") return capped();
  const { key } = made;
  return page("Your new key", `<p>Copy it now and paste it in Loft → Settings → Push notifications. <span class="warn">It is shown only this once:</span> if you lose it, revoke it and make another.</p><code>${esc(key)}</code><p class="note">Name: ${esc(name)}</p><div class="row"><a class="pill primary" href="/">I've copied it</a></div>`, 200, {}, { ...SIGNED_IN, layout: "card" });
}

/** POST /keys/revoke with `id` (the key's hash): deletes one of the signed-in account's keys. The relay refuses it from the next push. */
export async function revokeAccountKey(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const refused = crossSite(request, env);
  if (refused) return refused;
  const account = await accountOf(request, env, now);
  if (!account) return endSession(request);
  const id = String((await request.formData()).get("id") ?? "");
  const store = makeKv(env);
  const [mine] = /^[0-9a-f]{64}$/.test(id) ? await store.pipeline([["SISMEMBER", `account-keys:${account}`, id]]) : [0];
  if (mine !== 1) return page("No such key", '<p>That key isn\'t one of yours, or it\'s already revoked. <a href="/">Back</a></p>', 404, {}, SIGNED_IN);
  await store.transaction([["DEL", `relay-key:${id}`, `relay-key-used:${id}`], ["SREM", `account-keys:${account}`, id]]);
  return redirect("/");
}
