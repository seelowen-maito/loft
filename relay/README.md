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
  "device_token": "<the phone's APNs token, hex>",
  "environment": "production",
  "bundle_id": "<LOFT_BUNDLE_ID>",
  "payload": { "aps": { "alert": { "title": "…", "body": "…" } } },
  "priority": 10,
  "collapse_id": "permission:…"
}
```

- `environment` is `production` (TestFlight, App Store) or `development` (a debug build). It picks Apple's host.
- `priority` is 10 (time-sensitive, the default) or 5. `collapse_id` is at most 64 bytes.
- The payload is sent to Apple unchanged, with `apns-topic` set to the Loft bundle id.

| Answer | Meaning |
|---|---|
| `200 {"apple_status", "reason", "apns_id"}` | Apple answered. A `410`, or a `400` with `BadDeviceToken` or `DeviceTokenNotForTopic`, means the phone is gone: remove it. |
| `401` | No relay token, or one the relay didn't issue (or the operator revoked). |
| `403` | Another bundle id: use your own APNs key for your own build. |
| `413` | The payload is over 4 KB. |
| `400` | A malformed request. |
| `502` | Apple couldn't be reached. |

Every error body is `{"error": "…"}`.

## Settings (Vercel environment variables)

| | |
|---|---|
| `APNS_KEY_P8` | The `.p8` key's PEM text (`\n` escapes are accepted). |
| `APNS_KEY_ID`, `APNS_TEAM_ID` | Its key id and the Apple team id. |
| `LOFT_BUNDLE_ID` | The only bundle id served. |
| `KV_REST_API_URL`, `KV_REST_API_TOKEN` | Added when the KV (Upstash) store is connected to the project. |

`APNS_HOST_PRODUCTION` and `APNS_HOST_DEVELOPMENT` replace Apple's hosts. They are for the tests only; leave them unset.

## Deploy and test

Create a Vercel project with `relay/` as its root directory, connect a KV store, set the variables above and deploy.
To revoke an install, delete its `relay-token:<sha256>` key from the store.

`npm test` runs the tests with Node's own test runner (Node 22), against a fake Apple server and a fake store. No Apple key is needed.
