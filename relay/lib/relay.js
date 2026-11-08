// The push relay (ADR 0002): forwards the pushes of installs that hold a relay key to Apple. It only forwards:
// payloads and device tokens are never stored or logged, and only hashes of relay keys and device tokens are kept.
// Keys are made by the operator (scripts/create-key.js) or the portal; this file hands none out.

import { createHash, randomBytes } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { send } from "./apns.js";

/** Apple's limit for an alert's payload. */
export const MAX_PAYLOAD_BYTES = 4096;

/** What one relay key can ask for: [requests, window in seconds]. */
export const LIMITS = { pushesPerMinute: [20, 60], pushesPerDay: [1000, 86400] };

/** A relay key: `lrk_` and 32 random bytes, base64url. */
const KEY = /^lrk_[A-Za-z0-9_-]{43}$/;

const json = (status, body, headers) => Response.json(body, { status, headers });
const sha256 = (s) => createHash("sha256").update(s).digest("hex");
const hashed = (key) => `relay-key:${sha256(key)}`;

/** The most keys one portal account can hold. */
export const MAX_KEYS = 5;

/**
 * Counts one request against a fixed-window limit: null when it's within it, else a 429 saying when to try again.
 * `who` is already a hash, so the store never holds an IP or a token.
 */
async function overLimit(store, name, who, now) {
  const [max, seconds] = LIMITS[name];
  const window = Math.floor(now / 1000 / seconds);
  if ((await store.hit(`limit:${name}:${who}:${window}`, seconds)) <= max) return null;
  const retry = (window + 1) * seconds - Math.floor(now / 1000);
  return json(429, { error: `too many requests: at most ${max} per ${seconds === 60 ? "minute" : "day"}` }, { "retry-after": String(retry) });
}

/** Settings the relay can't work without. Without LOFT_BUNDLE_ID, for one, the bundle id check would pass anything. */
const REQUIRED = ["APNS_KEY_P8", "APNS_KEY_ID", "APNS_TEAM_ID", "LOFT_BUNDLE_ID", "KV_REST_API_URL", "KV_REST_API_TOKEN"];
const unconfigured = (env) => (REQUIRED.some((k) => !env[k]) ? json(500, { error: "the relay isn't configured" }) : null);

/** How long a key's last-used date is kept after its last push. It expires, so a push racing a revoke can't leave one forever. */
export const USED_SECONDS = 365 * 86400;

/** A new relay key for `label` (who it's for): the key, its hash (`id`, which names it without being it) and its record. */
export function newKey(label, now = Date.now(), account = undefined) {
  const key = `lrk_${randomBytes(32).toString("base64url")}`;
  return { key, id: sha256(key), record: JSON.stringify({ label, created: new Date(now).toISOString(), ...(account && { account }) }) };
}

/** For the operator's script: a new key, stored as its hash only. It's returned once and can't be read back. */
export async function createKey(env, label, now = Date.now()) {
  const { key, id, record } = newKey(label, now);
  await makeKv(env).set(`relay-key:${id}`, record);
  return key;
}

/** How long a device stays bound to its account after the last push to it. */
export const BOUND_SECONDS = 30 * 86400;

/**
 * Binds the device behind `token` to `owner` (an account, or an operator key with none) on the first push to it, and
 * keeps the binding while pushes keep coming: true when `owner` may push to it. The store keys it by the token's hash.
 * An account's devices are listed (`account-devices:<id>`) so deleting the account can unbind them.
 */
async function bound(store, token, owner, account) {
  const device = sha256(token);
  const [, holder] = await store.transaction([["SET", `device-owner:${device}`, owner, "NX", "EX", BOUND_SECONDS], ["GET", `device-owner:${device}`]]);
  if (holder !== owner) return false;
  const listed = account ? [["SADD", `account-devices:${account}`, device], ["EXPIRE", `account-devices:${account}`, BOUND_SECONDS]] : [];
  await store.transaction([["EXPIRE", `device-owner:${device}`, BOUND_SECONDS], ...listed]);
  return true;
}

/** Why a push request's body can't be forwarded, as the answer to give, or null when it can. */
function invalid(b, env) {
  const bad = (error) => json(400, { error });
  if (!b || typeof b !== "object") return bad("the body must be a JSON object");
  if (typeof b.device_token !== "string" || !/^[0-9a-fA-F]{64}$/.test(b.device_token)) return bad("device_token must be the phone's APNs token: 64 hex characters");
  if (b.environment !== "production" && b.environment !== "development") return bad('environment must be "production" or "development"');
  if (b.bundle_id !== env.LOFT_BUNDLE_ID) return json(403, { error: "the relay only serves the Loft app's bundle id; use your own APNs key for another app" });
  if (!b.payload || typeof b.payload !== "object" || Array.isArray(b.payload)) return bad("payload must be a JSON object");
  if (b.priority !== undefined && b.priority !== 5 && b.priority !== 10) return bad("priority must be 5 or 10");
  if (b.collapse_id !== undefined && (typeof b.collapse_id !== "string" || !/^[\x20-\x7e]{1,64}$/.test(b.collapse_id))) return bad("collapse_id must be 1-64 printable ASCII characters");
  return null;
}

/**
 * `POST /api/push`, with `Authorization: Bearer <relay key>` and `{device_token, environment, bundle_id, payload,
 * priority?, collapse_id?}`. When Apple answered: 200 `{apple_status, reason, apns_id}` (a 410, or a 400 with
 * BadDeviceToken or DeviceTokenNotForTopic, means the caller removes the phone). Refused by the relay: 4xx `{error}`,
 * 403 for a device bound to another account, 429 with Retry-After over a limit. Every authorized request counts,
 * forwarded or not.
 */
export async function push(request, env = process.env, now = Date.now(), appleTimeoutMs = undefined) {
  const broken = unconfigured(env);
  if (broken) return broken;
  const store = makeKv(env);
  const key = (request.headers.get("authorization") ?? "").match(/^Bearer (\S+)$/)?.[1];
  // Anything not shaped like a key is refused before it costs a store lookup.
  const record = key && KEY.test(key) ? await store.get(hashed(key)) : null;
  if (!record) return json(401, { error: "a valid relay key is required" });
  const who = sha256(key);
  const limited = (await overLimit(store, "pushesPerMinute", who, now)) ?? (await overLimit(store, "pushesPerDay", who, now));
  if (limited) return limited;
  await store.set(`relay-key-used:${who}`, new Date(now).toISOString(), USED_SECONDS);
  const tooBig = () => json(413, { error: `the payload must be at most ${MAX_PAYLOAD_BYTES} bytes` });
  // The whole request (payload plus a few small fields) can't be much over the payload's cap.
  if (Number(request.headers.get("content-length") ?? 0) > MAX_PAYLOAD_BYTES * 2) return tooBig();
  const raw = await request.text();
  if (Buffer.byteLength(raw) > MAX_PAYLOAD_BYTES * 2) return tooBig();
  let b;
  try {
    b = JSON.parse(raw);
  } catch {
    return json(400, { error: "the body must be JSON" });
  }
  const refused = invalid(b, env);
  if (refused) return refused;
  // The payload is forwarded exactly as it came.
  const body = JSON.stringify(b.payload);
  if (Buffer.byteLength(body) > MAX_PAYLOAD_BYTES) return tooBig();
  // A portal key pushes for its account, so its account's other Lofts share the phone; an operator key, for itself.
  let account;
  try {
    account = JSON.parse(record).account;
  } catch {}
  if (!(await bound(store, b.device_token, account ? `account:${account}` : `key:${who}`, account)))
    return json(403, { error: "this device belongs to another account: only that account's relay keys can push to it" });
  let apple;
  try {
    apple = await send(env, { deviceToken: b.device_token, environment: b.environment, priority: b.priority ?? 10, collapseId: b.collapse_id, body }, appleTimeoutMs);
  } catch (e) {
    // The kind of failure only: never the payload, the device token or the key.
    console.error(`push relay: ${e.key ? "the APNs key can't be used" : e.timeout ? "Apple timed out" : "Apple unreachable"} (${e.code ?? e.name})`);
    if (e.key) return json(500, { error: "the relay's Apple key can't be used" });
    return e.timeout ? json(504, { error: "Apple didn't answer in time" }) : json(502, { error: "couldn't reach Apple" });
  }
  // A stream that ended without a status never got Apple's answer.
  if (!apple.status) return json(502, { error: "couldn't reach Apple" });
  return json(200, { apple_status: apple.status, reason: apple.reason, apns_id: apple.apnsId });
}
