// Sends one push to Apple over HTTP/2 with token-based auth (ES256 provider token from the operator's .p8).

import { connect } from "node:http2";
import { createPrivateKey, sign } from "node:crypto";

const HOSTS = { production: "https://api.push.apple.com", development: "https://api.sandbox.push.apple.com" };

// Apple wants the provider token refreshed within an hour and no more than every 20 minutes.
const TOKEN_TTL_MS = 45 * 60 * 1000;
let cached = null;

function providerToken(env, now = Date.now()) {
  if (cached && cached.keyId === env.APNS_KEY_ID && now - cached.at < TOKEN_TTL_MS) return cached.jwt;
  const b64 = (v) => Buffer.from(typeof v === "string" ? v : JSON.stringify(v)).toString("base64url");
  const head = b64({ alg: "ES256", kid: env.APNS_KEY_ID });
  const claims = b64({ iss: env.APNS_TEAM_ID, iat: Math.floor(now / 1000) });
  const key = createPrivateKey(env.APNS_KEY_P8.replace(/\\n/g, "\n"));
  const sig = sign("sha256", Buffer.from(`${head}.${claims}`), { key, dsaEncoding: "ieee-p1363" }).toString("base64url");
  cached = { keyId: env.APNS_KEY_ID, at: now, jwt: `${head}.${claims}.${sig}` };
  return cached.jwt;
}

/** Apple's host for an environment. APNS_HOST_* override it for the tests' fake Apple, never in production. */
export const host = (env, environment) =>
  (env.VERCEL_ENV !== "production" && (environment === "development" ? env.APNS_HOST_DEVELOPMENT : env.APNS_HOST_PRODUCTION)) || HOSTS[environment];

/** How long Apple gets to answer one push. */
export const APPLE_TIMEOUT_MS = 10_000;

/** Sends `push` and resolves with Apple's answer: `{ status, reason, apnsId }`. Rejects when the key can't be used
 *  (the error's `key` is true), when Apple can't be reached or closes without answering, or when it doesn't answer
 *  within `timeoutMs` (`timeout` is true). */
export function send(env, push, timeoutMs = APPLE_TIMEOUT_MS) {
  let jwt;
  try {
    jwt = providerToken(env);
  } catch (e) {
    return Promise.reject(Object.assign(e, { key: true }));
  }
  return new Promise((resolve, reject) => {
    const session = connect(host(env, push.environment));
    let done = false;
    const fail = (e) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      session.destroy();
      reject(e);
    };
    const timer = setTimeout(() => fail(Object.assign(new Error("Apple didn't answer in time"), { timeout: true })), timeoutMs);
    session.on("error", fail);
    const headers = {
      ":method": "POST",
      ":path": `/3/device/${push.deviceToken}`,
      authorization: `bearer ${jwt}`,
      "apns-topic": env.LOFT_BUNDLE_ID,
      "apns-push-type": "alert",
      "apns-priority": String(push.priority),
      "content-type": "application/json",
    };
    if (push.collapseId) headers["apns-collapse-id"] = push.collapseId;
    const req = session.request(headers);
    let status = 0;
    let apnsId = null;
    let body = "";
    req.on("response", (h) => {
      status = h[":status"];
      apnsId = h["apns-id"] ?? null;
    });
    req.setEncoding("utf8");
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      session.close();
      let reason = null;
      try {
        reason = body ? JSON.parse(body).reason ?? null : null;
      } catch {
        // Apple's error bodies are JSON; anything else has no reason to pass on.
      }
      resolve({ status, reason, apnsId });
    });
    req.on("error", fail);
    // Closed without an answer (a GOAWAY, a reset): not Apple's 200.
    req.on("close", () => fail(new Error("Apple closed the stream without answering")));
    req.end(push.body);
  });
}
