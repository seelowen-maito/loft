// The push relay (ADR 0002): issues relay tokens and forwards an install's pushes to Apple. It only forwards:
// payloads and device tokens are never stored or logged, and only hashes of relay tokens are kept.

import { createHash, randomBytes } from "node:crypto";
import { kv as makeKv } from "./kv.js";
import { send } from "./apns.js";

/** Apple's limit for an alert's payload. */
export const MAX_PAYLOAD_BYTES = 4096;

const json = (status, body) => Response.json(body, { status });
const hashed = (token) => `relay-token:${createHash("sha256").update(token).digest("hex")}`;

/** `POST /api/token`: a new relay token. Only its hash is stored. */
export async function issueToken(_request, env = process.env) {
  const token = `lrt_${randomBytes(32).toString("base64url")}`;
  await makeKv(env).set(hashed(token), new Date().toISOString());
  return json(201, { token });
}

/** What's wrong with a push request's body, or null when it can be forwarded. */
function invalid(b, env) {
  if (!b || typeof b !== "object") return "the body must be a JSON object";
  if (typeof b.device_token !== "string" || !/^[0-9a-fA-F]{32,200}$/.test(b.device_token)) return "device_token must be the phone's APNs token in hex";
  if (b.environment !== "production" && b.environment !== "development") return 'environment must be "production" or "development"';
  if (b.bundle_id !== env.LOFT_BUNDLE_ID) return "the relay only serves the Loft app's bundle id; use your own APNs key for another app";
  if (!b.payload || typeof b.payload !== "object" || Array.isArray(b.payload)) return "payload must be a JSON object";
  if (b.priority !== undefined && b.priority !== 5 && b.priority !== 10) return "priority must be 5 or 10";
  if (b.collapse_id !== undefined && (typeof b.collapse_id !== "string" || !b.collapse_id || Buffer.byteLength(b.collapse_id) > 64)) return "collapse_id must be 1-64 bytes";
  return null;
}

/**
 * `POST /api/push`, with `Authorization: Bearer <relay token>` and `{device_token, environment, bundle_id, payload,
 * priority?, collapse_id?}`. When Apple answered: 200 `{apple_status, reason, apns_id}` (a 410, or a 400 with
 * BadDeviceToken or DeviceTokenNotForTopic, means the caller removes the phone). Refused by the relay: 4xx `{error}`.
 */
export async function push(request, env = process.env) {
  const token = (request.headers.get("authorization") ?? "").match(/^Bearer (\S+)$/)?.[1];
  if (!token || !(await makeKv(env).exists(hashed(token)))) return json(401, { error: "a valid relay token is required" });
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
    apple = await send(env, { deviceToken: b.device_token, environment: b.environment, priority: b.priority ?? 10, collapseId: b.collapse_id, body });
  } catch {
    return json(502, { error: "couldn't reach Apple" });
  }
  return json(200, { apple_status: apple.status, reason: apple.reason, apns_id: apple.apnsId });
}
