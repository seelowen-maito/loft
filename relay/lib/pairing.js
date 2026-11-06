// Signing a Loft in (ADR 0003): a Loft asks for a one-time code, the user approves it on the portal while signed in,
// and the Loft that asked collects a new relay key once. Codes live 10 minutes and are dead after the key is
// collected. The key waits in the store, readable only with the code, until then.

import { createHash, randomBytes } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { createKey } from "./relay.js";
import { REQUIRED, accountOf, cookie, cookieOf, esc, notConfigured, page, redirect } from "./portal.js";

const CODE_SECONDS = 600;
/** Codes one IP can ask for per hour. */
const CODES_PER_HOUR = 5;
const CODE = /^[A-Za-z0-9_-]{22}$/;

const sha256 = (s) => createHash("sha256").update(s).digest("hex");
const json = (status, body) => Response.json(body, { status, headers: { "cache-control": "no-store" } });
const waiting = (code) => `pair:${sha256(code)}`;
const approved = (code) => `pair-key:${sha256(code)}`;
const claimed = (code) => `pair-claim:${sha256(code)}`;

/** POST /api/pair: a new code for a Loft, `{code, expires_in}`. Limited per IP; the store holds only a hash of the IP. */
export async function pair(request, env = process.env, now = Date.now()) {
  if (!env.KV_REST_API_URL || !env.KV_REST_API_TOKEN) return json(500, { error: "the relay isn't configured" });
  const store = makeKv(env);
  const ip = (request.headers.get("x-forwarded-for") ?? "").split(",")[0].trim();
  if ((await store.hit(`limit:pair:${sha256(ip)}:${Math.floor(now / 3600000)}`, 3600)) > CODES_PER_HOUR) return json(429, { error: `at most ${CODES_PER_HOUR} sign-ins per hour` });
  const code = randomBytes(16).toString("base64url");
  await store.setEx(waiting(code), "1", CODE_SECONDS);
  return json(200, { code, expires_in: CODE_SECONDS });
}

/** POST /api/pair-poll with `{code}`: 200 `{key}` once, after the user approved it; 202 while it waits; else 404. */
export async function poll(request, env = process.env) {
  if (!env.KV_REST_API_URL || !env.KV_REST_API_TOKEN) return json(500, { error: "the relay isn't configured" });
  const code = (await request.json().catch(() => null))?.code;
  if (typeof code !== "string" || !CODE.test(code)) return json(404, { error: "unknown or expired code" });
  const store = makeKv(env);
  const key = await store.getdel(approved(code));
  if (key) return json(200, { key });
  if (await store.exists(waiting(code))) return json(202, { status: "waiting" });
  return json(404, { error: "unknown or expired code" });
}

/** GET /connect?code=: the page where the user allows a Loft: sign in first, then Allow. */
export async function connect(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const code = new URL(request.url).searchParams.get("code") ?? "";
  if (!CODE.test(code) || !(await makeKv(env).exists(waiting(code)))) return page("Code expired", '<p>This code has expired or was already used. Press Sign in in Loft again.</p>', 404);
  if (!accountOf(request, env, now)) {
    return page("Connect your Loft", '<p>Sign in with Apple to let your Loft send push notifications.</p><p><a href="/auth/start"><button>Sign in with Apple</button></a></p>', 200, { "set-cookie": cookie("pair", code, CODE_SECONDS) });
  }
  return page(
    "Connect your Loft",
    `<form method="post" action="/connect"><input type="hidden" name="code" value="${esc(code)}"><p><label>Key name <input name="name" maxlength="60" required value="My Mac"></label></p><p>Allow this Loft to send you push notifications? Only allow if you just pressed Sign in in Loft; if someone sent you this link, close the page.</p><button>Allow</button></form>`,
    200,
    { "set-cookie": cookie("pair", "", 0) },
  );
}

/** POST /connect with `code` and `name`: creates a key for the signed-in account and hands it to the Loft that asked. */
export async function approve(request, env = process.env, now = Date.now()) {
  if (REQUIRED.some((k) => !env[k])) return notConfigured();
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const form = await request.formData();
  const code = String(form.get("code") ?? "");
  const name = String(form.get("name") ?? "").trim().slice(0, 60);
  if (!name) return page("Name the key", '<p>A key needs a name. Go back and try again.</p>', 400);
  const store = makeKv(env);
  // The first approval claims the code; a second click or tab finds it claimed. The waiting marker stays until the key
  // is stored, so a poll in between still answers 202 instead of 404.
  const expired = () => page("Code expired", '<p>This code has expired or was already used. Press Sign in in Loft again.</p>', 404);
  if (!CODE.test(code) || !(await store.exists(waiting(code)))) return expired();
  if ((await store.hit(claimed(code), CODE_SECONDS)) > 1) return expired();
  await store.setEx(approved(code), await createKey(env, name, now, account), CODE_SECONDS);
  await store.getdel(waiting(code));
  return page("Loft is signed in", "<p>Your Loft has its key. You can close this page.</p>");
}
