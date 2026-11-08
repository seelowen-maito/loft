// Signing a Loft in (ADR 0003): a Loft asks for a one-time code, the user approves it on the portal while signed in,
// and the Loft that asked collects a new relay key once. Codes live 10 minutes and are dead after the key is
// collected. The key waits in the store, readable only with the code, until then.
//
// The code is in the portal link, so a link sent to someone else would let them allow the sender's Loft on their own
// account. So each code has a short confirmation code that only the Loft that asked shows: the user types it on the
// Allow page, and a few wrong tries end the code.

import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { incomplete, accountOf, appleButton, capped, cookie, cookieOf, esc, limited, makeAccountKey, notConfigured, page, redirect } from "./portal.js";

const CODE_SECONDS = 600;
/** Codes one IP can ask for per hour. */
const CODES_PER_HOUR = 5;
const CODE = /^[A-Za-z0-9_-]{22}$/;
const SECRET = CODE;
/** Wrong confirmation codes before the sign-in code is dead. */
export const CONFIRM_TRIES = 5;
// No 0/O, 1/I/L: easy to read off one screen and type into another.
const CONFIRM_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";

const sha256 = (s) => createHash("sha256").update(s).digest("hex");
const json = (status, body) => Response.json(body, { status, headers: { "cache-control": "no-store" } });
const waiting = (code) => `pair:${sha256(code)}`;
// The code is in the portal link, so anyone who sees the link knows it. The key is parked under the hash of a secret
// only the Loft that asked holds, and that Loft alone can collect it.
const approved = (secretHash) => `pair-key:${secretHash}`;
const claimed = (code) => `pair-claim:${sha256(code)}`;
const wrongTries = (code) => `pair-tries:${sha256(code)}`;
/** The 6-character confirmation code, derived from the hash of the Loft's secret: the link alone doesn't give it. */
const confirmationOf = (secretHash) =>
  [...createHash("sha256").update(`confirm.${secretHash}`).digest().subarray(0, 6)].map((b) => CONFIRM_ALPHABET[b % CONFIRM_ALPHABET.length]).join("");
/** What the user typed, as a code: case, spaces and dashes don't matter. */
const typed = (s) => String(s ?? "").toUpperCase().replace(/[\s-]/g, "");
const same = (a, b) => {
  const [x, y] = [Buffer.from(a), Buffer.from(b)];
  return x.length === y.length && timingSafeEqual(x, y);
};

/** POST /api/pair: a new code, secret and confirmation code for a Loft, `{code, secret, confirm, expires_in}`. Limited per IP; the store holds only a hash of the IP. */
export async function pair(request, env = process.env, now = Date.now()) {
  if (!env.KV_REST_API_URL || !env.KV_REST_API_TOKEN) return json(500, { error: "the relay isn't configured" });
  const store = makeKv(env);
  const ip = (request.headers.get("x-forwarded-for") ?? "").split(",")[0].trim();
  if ((await store.hit(`limit:pair:${sha256(ip)}:${Math.floor(now / 3600000)}`, 3600)) > CODES_PER_HOUR) return json(429, { error: `at most ${CODES_PER_HOUR} sign-ins per hour` });
  const code = randomBytes(16).toString("base64url");
  const secret = randomBytes(16).toString("base64url");
  // The waiting marker holds the secret's hash and when the code ends.
  await store.setEx(waiting(code), `${sha256(secret)}.${Math.floor(now / 1000) + CODE_SECONDS}`, CODE_SECONDS);
  return json(200, { code, secret, confirm: confirmationOf(sha256(secret)), expires_in: CODE_SECONDS });
}

/** POST /api/pair-poll with `{code, secret}`: 200 `{key}` once, after the user approved it; 202 while it waits; else 404. */
export async function poll(request, env = process.env) {
  if (!env.KV_REST_API_URL || !env.KV_REST_API_TOKEN) return json(500, { error: "the relay isn't configured" });
  const body = await request.json().catch(() => null);
  const { code, secret } = body ?? {};
  if (typeof code !== "string" || !CODE.test(code) || typeof secret !== "string" || !SECRET.test(secret)) return json(404, { error: "unknown or expired code" });
  const store = makeKv(env);
  const key = await store.getdel(approved(sha256(secret)));
  if (key) return json(200, { key });
  if (((await store.get(waiting(code))) ?? "").split(".")[0] === sha256(secret)) return json(202, { status: "waiting" });
  return json(404, { error: "unknown or expired code" });
}

const expiredPage = () => page("Code expired", '<p>This code has expired or was already used. Press Sign in in Loft again.</p>', 404);

/** The Allow form: the key's name and the confirmation code Loft shows, which this page never shows itself. */
const allowForm = (code, name = "My Mac", error = "", status = 200, headers = {}) =>
  page(
    "Connect your Loft",
    `<form method="post" action="/connect"><input type="hidden" name="code" value="${esc(code)}">` +
      (error ? `<p class="bad">${esc(error)}</p>` : "") +
      `<p><label>Confirmation code<input name="confirm" maxlength="12" required autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="The 6 characters Loft shows"></label></p>` +
      `<p><label>Key name<input name="name" maxlength="60" required value="${esc(name)}"></label></p>` +
      `<p>Type the code shown in Loft, under Settings → Push notifications, next to Sign in. Only allow if you just pressed Sign in in Loft; if someone sent you this link, close the page.</p><button class="primary">Allow</button></form>`,
    status,
    headers,
  );

/** GET /connect?code=: the page where the user allows a Loft: sign in first, then type the confirmation code and Allow. */
export async function connect(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const code = new URL(request.url).searchParams.get("code") ?? "";
  if (!CODE.test(code) || !(await makeKv(env).exists(waiting(code)))) return expiredPage();
  if (!accountOf(request, env, now)) {
    return page("Connect your Loft", `<p>Sign in with Apple to let your Loft send push notifications.</p><p>${appleButton}</p>`, 200, { "set-cookie": cookie("pair", code, CODE_SECONDS) });
  }
  return allowForm(code, undefined, "", 200, { "set-cookie": cookie("pair", "", 0) });
}

/** POST /connect with `code`, `confirm` and `name`: once the confirmation code matches, creates a key for the signed-in
 *  account and hands it to the Loft that asked. */
export async function approve(request, env = process.env, now = Date.now()) {
  if (incomplete(env)) return notConfigured(env);
  const account = accountOf(request, env, now);
  if (!account) return redirect("/");
  const form = await request.formData();
  const code = String(form.get("code") ?? "");
  const name = String(form.get("name") ?? "").trim().slice(0, 60);
  if (!name) return page("Name the key", '<p>A key needs a name. Go back and try again.</p>', 400);
  const store = makeKv(env);
  // The first approval claims the code; a second click or tab finds it claimed. The waiting marker stays until the key
  // is stored, so a poll in between still answers 202 instead of 404.
  const marker = CODE.test(code) ? await store.get(waiting(code)) : null;
  if (!marker) return expiredPage();
  const [secretHash, ends] = marker.split(".");
  if (!same(typed(form.get("confirm")), confirmationOf(secretHash))) {
    const wrong = await store.hit(wrongTries(code), CODE_SECONDS);
    if (wrong >= CONFIRM_TRIES) {
      await store.del(waiting(code));
      return page("Too many wrong codes", "<p>That code was wrong too many times, so this sign-in has ended. Press Sign in in Loft again.</p>", 403);
    }
    const left = CONFIRM_TRIES - wrong;
    return allowForm(code, name, `That isn't the code Loft shows. ${left} ${left === 1 ? "try" : "tries"} left.`, 400);
  }
  const limit = await limited(store, request, "key", now);
  if (limit) return limit;
  if ((await store.hit(claimed(code), CODE_SECONDS)) > 1) return expiredPage();
  // The key waits no longer than the code would have.
  const left = Math.max(1, Number(ends) - Math.floor(now / 1000));
  const made = await makeAccountKey(store, account, name, now);
  if (made.error === "gone") return redirect("/", cookie("session", "", 0));
  if (made.error === "capped") {
    await store.del(claimed(code)); // the code is still good once the user has revoked a key
    return capped();
  }
  await store.setEx(approved(secretHash), made.key, left);
  await store.getdel(waiting(code));
  return page("Loft is signed in", "<p>Your Loft has its key. You can close this page.</p>");
}
