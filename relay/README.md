# Push relay

Forwards Loft's pushes to Apple for installs that hold a **relay key** and have no APNs key of their own (ADR 0002).
It holds the operator's Apple key, serves only the official Loft app's bundle id, and only forwards: payloads and
device tokens are never stored or logged, and only hashes of relay keys are kept. It's login-only: there's no endpoint
that hands out keys. The operator makes them with `scripts/create-key.js`; later the push portal (Sign in with Apple)
will.

Plain JavaScript Vercel functions with no dependencies and no build step. The store is Upstash Redis (Vercel KV) over
its REST API.

## API

`POST /api/push` with `Authorization: Bearer <relay key>`:

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
| `401` | No relay key, one not shaped like a key (`lrk_` and 43 characters), or one the store doesn't have (never made, or revoked). |
| `403` | Another bundle id: use your own APNs key for your own build. |
| `413` | The payload is over 4 KB. |
| `429` | Over a limit; `Retry-After` says in how many seconds to try again. |
| `400` | A malformed request. |
| `502` | Apple couldn't be reached, or closed without answering. |
| `504` | Apple didn't answer within 10 seconds. |
| `500` | The relay is missing one of its settings below, or its Apple key can't be used. |

Every error body is `{"error": "…"}`.

## Limits

20 pushes per minute and 1,000 per day per relay key. They're fixed windows (the clock minute and the UTC day),
counted in the store under the key's hash. Every authorized request counts, whether or not it's forwarded.

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
   Name the project `loft-push-relay`. Loft's relay URL is `https://loft-push-relay.vercel.app`
   (`RELAY_URL` in `backend/src/apns.rs`); with another name, change that constant.
2. Storage → connect an Upstash Redis (KV) store to the project. It adds `KV_REST_API_URL` and `KV_REST_API_TOKEN`.
3. Settings → Environment Variables (Production), all marked **Sensitive**: `APNS_KEY_P8` (paste the whole `.p8`),
   `APNS_KEY_ID`, `APNS_TEAM_ID`, `LOFT_BUNDLE_ID`. Nothing secret goes in the repo.
4. Deploy, make a relay key (below), paste it in a Loft with no APNs key (Settings → Push notifications → Loft relay)
   and press Send test push.

`vercel.json` caps each call at 15 seconds.

## Runbook: make a relay key

With the store's credentials in your shell (Vercel → Storage → the store → `.env.local` tab):

```sh
cd relay
KV_REST_API_URL=… KV_REST_API_TOKEN=… node scripts/create-key.js "Sam (sam@example.com)"
```

It prints the key once; only its hash is stored, with the label and the date. Send the key to that person privately.
They paste it in Loft → Settings → Push notifications.

## Runbook: revoke a relay key

The store holds `relay-key:<sha256 of the key>` with its label, never the key itself. To stop one:

1. **Find it.** In the store's Data Browser, search `relay-key:*` and open them: the value has the label it was made
   for. Or, from the key itself: `printf %s 'lrk_…' | shasum -a 256`. For a busy one, the counters
   `limit:pushesPerDay:<hash>:<day>` name its hash.
2. **Delete** `relay-key:<hash>` in the Data Browser (or `DEL relay-key:<hash>` in its CLI).
3. From then on that Loft gets `401` and its pushes stop. To let that person back in, make them a new key.

To stop the whole relay, remove the project's `APNS_KEY_P8`, or revoke the key in the Apple Developer portal.

## Test

`npm test` runs the tests with Node's own test runner (Node 22), against a fake Apple server and a fake store. No
Apple key is needed.
