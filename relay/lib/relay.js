// The push relay (ADR 0002): issues relay tokens and forwards an install's pushes to Apple. It only forwards:
// payloads and device tokens are never stored or logged, and only hashes of relay tokens are kept.

import { createHash, randomBytes } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { send } from "./apns.js";

/** Apple's limit for an alert's payload. */
export const MAX_PAYLOAD_BYTES = 4096;

/** What one IP or one install can ask for: [requests, window in seconds]. */
export const LIMITS = { tokensPerIp: [5, 3600], pushesPerMinute: [20, 60], pushesPerDay: [1000, 86400] };

const json = (status, body, headers) => Response.json(body, { status, headers });
const sha256 = (s) => createHash("sha256").update(s).digest("hex");
const hashed = (token) => `relay-token:${sha256(token)}`;

/**
 * Counts one request against a fixed-window limit: null when it's within it, else a 429 saying when to try again.
 * `who` is already a hash, so the store never holds an IP or a token.
 */
async function overLimit(store, name, who, now) {
  const [max, seconds] = LIMITS[name];
  const window = Math.floor(now / 1000 / seconds);
  if ((await store.hit(`limit:${name}:${who}:${window}`, seconds)) <= max) return null;
  const retry = (window + 1) * seconds - Math.floor(now / 1000);
  return json(429, { error: `too many requests: at most ${max} per ${seconds === 60 ? "minute" : seconds === 3600 ? "hour" : "day"}` }, { "retry-after": String(retry) });
}

/** Settings the relay can't work without. Without LOFT_BUNDLE_ID, for one, the bundle id check would pass anything. */
const REQUIRED = ["APNS_KEY_P8", "APNS_KEY_ID", "APNS_TEAM_ID", "LOFT_BUNDLE_ID", "KV_REST_API_URL", "KV_REST_API_TOKEN"];
const unconfigured = (env) => (REQUIRED.some((k) => !env[k]) ? json(500, { error: "the relay isn't configured" }) : null);

/** The caller's IP: Vercel puts the client first in x-forwarded-for. */
const clientIp = (request) => (request.headers.get("x-forwarded-for") ?? "").split(",")[0].trim() || "unknown";

/** `POST /api/token`: a new relay token. Only its hash is stored. */
export async function issueToken(request, env = process.env, now = Date.now()) {
  const broken = unconfigured(env);
  if (broken) return broken;
  const store = makeKv(env);
  const limited = await overLimit(store, "tokensPerIp", sha256(`ip:${clientIp(request)}`), now);
  if (limited) return limited;
  const token = `lrt_${randomBytes(32).toString("base64url")}`;
  await store.set(hashed(token), new Date(now).toISOString());
  return json(201, { token });
}

/** What's wrong with a push request's body, or null when it can be forwarded. */
function invalid(b, env) {
  if (!b || typeof b !== "object") return "the body must be a JSON object";
  if (typeof b.device_token !== "string" || !/^[0-9a-fA-F]{64}$/.test(b.device_token)) return "device_token must be the phone's APNs token: 64 hex characters";
  if (b.environment !== "production" && b.environment !== "development") return 'environment must be "production" or "development"';
  if (b.bundle_id !== env.LOFT_BUNDLE_ID) return "the relay only serves the Loft app's bundle id; use your own APNs key for another app";
  if (!b.payload || typeof b.payload !== "object" || Array.isArray(b.payload)) return "payload must be a JSON object";
  if (b.priority !== undefined && b.priority !== 5 && b.priority !== 10) return "priority must be 5 or 10";
  if (b.collapse_id !== undefined && (typeof b.collapse_id !== "string" || !/^[\x20-\x7e]{1,64}$/.test(b.collapse_id))) return "collapse_id must be 1-64 printable ASCII characters";
  return null;
}

/**
 * `POST /api/push`, with `Authorization: Bearer <relay token>` and `{device_token, environment, bundle_id, payload,
 * priority?, collapse_id?}`. When Apple answered: 200 `{apple_status, reason, apns_id}` (a 410, or a 400 with
 * BadDeviceToken or DeviceTokenNotForTopic, means the caller removes the phone). Refused by the relay: 4xx `{error}`,
 * 429 with Retry-After over a limit. Every authorized request counts, forwarded or not.
 */
export async function push(request, env = process.env, now = Date.now(), appleTimeoutMs = undefined) {
  const broken = unconfigured(env);
  if (broken) return broken;
  const store = makeKv(env);
  const token = (request.headers.get("authorization") ?? "").match(/^Bearer (\S+)$/)?.[1];
  if (!token || !(await store.exists(hashed(token)))) return json(401, { error: "a valid relay token is required" });
  const who = sha256(token);
  const limited = (await overLimit(store, "pushesPerMinute", who, now)) ?? (await overLimit(store, "pushesPerDay", who, now));
  if (limited) return limited;
  const raw = await request.text();
  if (Buffer.byteLength(raw) > MAX_PAYLOAD_BYTES * 2) return json(413, { error: `the payload must be at most ${MAX_PAYLOAD_BYTES} bytes` });
  let b;
  try {
    b = JSON.parse(raw);
  } catch {
    return json(400, { error: "the body must be JSON" });
  }
  const why = invalid(b, env);
  if (why) return json(why.includes("bundle id") ? 403 : 400, { error: why });
  // The payload is forwarded exactly as it came.
  const body = JSON.stringify(b.payload);
  if (Buffer.byteLength(body) > MAX_PAYLOAD_BYTES) return json(413, { error: `the payload must be at most ${MAX_PAYLOAD_BYTES} bytes` });
  let apple;
  try {
    apple = await send(env, { deviceToken: b.device_token, environment: b.environment, priority: b.priority ?? 10, collapseId: b.collapse_id, body }, appleTimeoutMs);
  } catch (e) {
    return e.timeout ? json(504, { error: "Apple didn't answer in time" }) : json(502, { error: "couldn't reach Apple" });
  }
  // A stream that ended without a status never got Apple's answer.
  if (!apple.status) return json(502, { error: "couldn't reach Apple" });
  return json(200, { apple_status: apple.status, reason: apple.reason, apns_id: apple.apnsId });
}
