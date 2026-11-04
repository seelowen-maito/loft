# Push relay

Forwards Loft's pushes to Apple for installs that have no APNs key of their own (ADR 0002). It holds the operator's
Apple key, serves only the official Loft app's bundle id, and only forwards: payloads and device tokens are never
stored or logged, and only hashes of relay tokens are kept.

Plain JavaScript Vercel functions with no dependencies and no build step. The store is Upstash Redis (Vercel KV) over its REST API.

## API

`POST /api/token` → `201 {"token": "lrt_…"}`. The install keeps it; it is the install's identity. Only its SHA-256 is stored.

`POST /api/push` with `Authorization: Bearer <relay token>`:

```json
{
  "device_token": "<the phone's APNs token, 64 hex characters>",
  "environment": "production",
  "bundle_id": "<LOFT_BUNDLE_ID>",
  "payload": { "aps": { "alert": { "title": "…", "body": "…" } } },
  "priority": 10,
  "collapse_id": "permission:…"
}
```

- `environment` is `production` (TestFlight, App Store) or `development` (a debug build). It picks Apple's host.
- `priority` is 10 (time-sensitive, the default) or 5. `collapse_id` is 1-64 printable ASCII characters.
- The payload is sent to Apple unchanged, with `apns-topic` set to the Loft bundle id.

| Answer | Meaning |
|---|---|
| `200 {"apple_status", "reason", "apns_id"}` | Apple answered. A `410`, or a `400` with `BadDeviceToken` or `DeviceTokenNotForTopic`, means the phone is gone: remove it. |
| `401` | No relay token, or one the relay didn't issue (or the operator revoked). |
| `403` | Another bundle id: use your own APNs key for your own build. |
| `413` | The payload is over 4 KB. |
| `400` | A malformed request. |
| `502` | Apple couldn't be reached. |
| `504` | Apple didn't answer within 10 seconds. |
| `500` | The relay is missing one of its settings below; it refuses everything until they're set. |
| `429` | Over a limit; `Retry-After` says in how many seconds to try again. |

Every error body is `{"error": "…"}`.

## Limits

| | |
|---|---|
| New relay tokens | 5 per IP per hour |
| Pushes | 20 per minute and 1,000 per day per relay token |

These are fixed windows (the clock minute, hour and UTC day), counted in the store under hashes of the IP and the
token. Every authorized push request counts, whether or not it's forwarded.

## Settings (Vercel environment variables)

| | |
|---|---|
| `APNS_KEY_P8` | The `.p8` key's PEM text (`\n` escapes are accepted). |
| `APNS_KEY_ID`, `APNS_TEAM_ID` | Its key id and the Apple team id. |
| `LOFT_BUNDLE_ID` | The only bundle id served. |
| `KV_REST_API_URL`, `KV_REST_API_TOKEN` | Added when the KV (Upstash) store is connected to the project. |

`APNS_HOST_PRODUCTION` and `APNS_HOST_DEVELOPMENT` replace Apple's hosts for the tests. They're ignored when
`VERCEL_ENV` is `production`.

## Deploy

1. Vercel → Add New → Project → this repo, with **Root Directory** `relay` and no framework or build command.
   Name the project `loft-push-relay`. Loft's default relay URL is `https://loft-push-relay.vercel.app`
   (`RELAY_URL` in `backend/src/apns.rs`); with another name, change that constant.
2. Storage → connect an Upstash Redis (KV) store to the project. It adds `KV_REST_API_URL` and `KV_REST_API_TOKEN`.
3. Settings → Environment Variables (Production), all marked **Sensitive**: `APNS_KEY_P8` (paste the whole `.p8`),
   `APNS_KEY_ID`, `APNS_TEAM_ID`, `LOFT_BUNDLE_ID`. Nothing secret goes in the repo.
4. Deploy. Check it: `curl -X POST https://loft-push-relay.vercel.app/api/token` answers `201` with a token. A Loft
   with no key then pushes through it: Settings → Push notifications → Send test push.

`vercel.json` caps each call at 15 seconds, so a slow Apple can't hold a function longer.

## Runbook: revoke a relay token

A relay token can't be read back from the store: only `relay-token:<sha256 of the token>` is there. To stop an install:

1. **Find the hash.**
   - If the install's owner sends you their token (it's in `~/Library/Application Support/Loft/apns/relay-token` on
     their Mac): `printf %s 'lrt_…' | shasum -a 256`.
   - If you're stopping an install that's misbehaving: in the store's Data Browser, look at the counters
     `limit:pushesPerDay:<hash>:<day>` and `limit:pushesPerMinute:<hash>:<minute>`. The `<hash>` of the busy ones is
     the token's hash.
2. **Delete** `relay-token:<hash>` in the Data Browser (or `DEL relay-token:<hash>` in its CLI).
3. From then on that install gets `401` and its pushes stop. Loft doesn't fetch a new token by itself. If the owner
   should get back in, they delete their `relay-token` file and Loft asks for a new one on its next push. The limit of
   5 new tokens per IP per hour still applies.

To stop the whole relay, remove the project's `APNS_KEY_P8`, or revoke the key in the Apple Developer portal.

## Test

`npm test` runs the tests with Node's own test runner (Node 22), against a fake Apple server and a fake store. No Apple key is needed.
