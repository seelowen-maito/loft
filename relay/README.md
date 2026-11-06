# Push relay

Forwards Loft's pushes to Apple for installs that hold a **relay key** and have no APNs key of their own (ADR 0002).
It holds the operator's Apple key, serves only the official Loft app's bundle id, and only forwards: payloads and
device tokens are never stored or logged, and only hashes of relay keys are kept. It's login-only: a key comes from
the **push portal** (below), after Sign in with Apple, or from the operator's `scripts/create-key.js`.

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

## The push portal (ADR 0003)

A web page in the same project. `/` offers **Sign in with Apple**; Apple's answer comes back to `/auth/callback`, and
the account is Apple's user id (`account:<id>` in the store) and nothing else: no email or name is requested. Signed
in, the user names a key and gets it once on the next page; only its hash is stored (with the name, the date and the
account), and the relay accepts it like one made by the script. The page lists the account's keys by name, with
their created and last-used dates (`relay-key-used:<hash>`, set on each authorized push and kept for a year), never the key. **Revoke**
deletes a key, so its next push gets `401`. An account holds at most 5 keys: a 6th is refused until one is revoked.
From one address (the first `x-forwarded-for` entry, stored only as a hash), at most 5 sign-ins (`/auth/start` and
Delete my account), 5 returns from Apple (`/auth/callback`) and 5 key creations (`/keys` and Allow on `/connect`) are
taken per hour: the 6th gets `429` with `Retry-After` until the next hour. These limits, and `/api/pair`'s, trust
`x-forwarded-for`: Vercel sets it. Hosting the relay anywhere else, put it behind a proxy that overwrites that header
with the caller's address, or anyone can set it and skip the limits. Requests without it all share one count.
**Delete my account** (`/account/delete`) asks the user to sign in with Apple once more: only the account's own Apple
ID confirms it. Then the account, its keys and their last-used dates are deleted, and the token Apple just gave is
revoked (`/auth/revoke`), which ends Sign in with Apple for the portal. No Apple token is ever stored. If Apple
refuses the revocation, the account is still deleted and the page says how to remove the portal in the Apple ID
settings. Without the portal's settings below, the page says
"The push portal isn't configured" and the relay works as before: the portal is optional.

**Sign in from Loft.** `POST /api/pair` gives a Loft a one-time code (`{code, secret, expires_in: 600}`, at most 5 per IP per
hour). Loft opens `<PORTAL_URL>/connect?code=…`: signed out, the page offers Sign in with Apple and brings the user back;
signed in, **Allow** makes a key for the account and parks it under the hash of the secret (the code is in the link; the secret never leaves the Loft). `POST /api/pair-poll` with `{code, secret}`
answers `202` while waiting, `200 {key}` exactly once after Allow, then `404` (also for an expired or unknown code). Until
it's collected, the new key waits in the store, readable only with the secret, for at most the code's 10 minutes.

`APPLE_ID_HOST` replaces Apple's sign-in host for the tests, and is ignored when `VERCEL_ENV` is `production`.

## Settings (Vercel environment variables)

The relay:

| | |
|---|---|
| `APNS_KEY_P8` | The push `.p8` key's PEM text (`\n` escapes are accepted). |
| `APNS_KEY_ID`, `APNS_TEAM_ID` | Its key id and the Apple team id. |
| `LOFT_BUNDLE_ID` | The only bundle id served. |
| `KV_REST_API_URL`, `KV_REST_API_TOKEN` | Added when the KV (Upstash) store is connected to the project. |

The portal (all of them, or it says "The push portal isn't configured"):

| | |
|---|---|
| `APPLE_SERVICES_ID` | The Services ID (the web client id), like `com.example.loft.portal`. |
| `APPLE_TEAM_ID` | The team id: the same as `APNS_TEAM_ID`. |
| `APPLE_SIGNIN_KEY_ID`, `APPLE_SIGNIN_KEY_P8` | The Sign in with Apple key's id and PEM text. A different key from the push key. |
| `PORTAL_URL` | The portal's address, exactly as entered in the Services ID, like `https://loft-push-relay.vercel.app`. |
| `SESSION_SECRET` | A long random string that signs the portal's cookies (`openssl rand -base64 32`). |

`APNS_HOST_PRODUCTION` and `APNS_HOST_DEVELOPMENT` replace Apple's hosts for the tests. They're ignored when
`VERCEL_ENV` is `production`.

## Runbook: deploy the relay and the portal

Everything below uses your own Apple developer account, Vercel account and domain: none of these values are in the
repo, so anyone can run their own.

**Apple (developer.apple.com → Certificates, Identifiers & Profiles)**

1. **App ID.** Identifiers → the Loft app's App ID (`LOFT_BUNDLE_ID`) → tick **Push Notifications** and **Sign in with
   Apple** (as a primary App ID) → Save.
2. **Push key.** Keys → **+** → tick **Apple Push Notifications service (APNs)** → download the `.p8` (only once) and
   note its Key ID. This is `APNS_KEY_P8` and `APNS_KEY_ID`.
3. **Services ID.** Identifiers → **+** → Services IDs → a description and an identifier like `com.example.loft.portal`
   (`APPLE_SERVICES_ID`) → Register. Open it, tick **Sign in with Apple** → Configure: primary App ID from step 1,
   **Domains** the portal's host (`loft-push-relay.vercel.app`, or your own domain), **Return URLs**
   `<PORTAL_URL>/auth/callback` → Save. Apple refuses `localhost` and IP addresses. If it refuses the `vercel.app`
   host, give the project your own domain (Vercel → Settings → Domains) and use that everywhere instead.
4. **Sign-in key.** Keys → **+** → tick **Sign in with Apple** → Configure with the App ID from step 1 → download the
   `.p8` and note its Key ID. This is `APPLE_SIGNIN_KEY_P8` and `APPLE_SIGNIN_KEY_ID`.
5. **Team ID**, top right of the portal: `APNS_TEAM_ID` and `APPLE_TEAM_ID`.

**Vercel**

6. Add New → Project → this repo, with **Root Directory** `relay` and no framework or build command. Name it, for
   example, `loft-push-relay`. `vercel.json` routes the portal's pages and caps each call at 15 seconds.
7. Storage → connect an Upstash Redis (KV) store to the project. It adds `KV_REST_API_URL` and `KV_REST_API_TOKEN`.
8. Settings → Environment Variables (Production), all marked **Sensitive**: every setting in the two tables above.
   Paste each `.p8` whole. Nothing secret goes in the repo.
9. Deploy.

**Loft**

10. Settings → Push notifications → **Relay address** (`https://<project>.vercel.app`) and **Portal address**
    (`PORTAL_URL`). Or `LOFT_RELAY_URL` / `LOFT_PORTAL_URL` for a scripted install. `RELAY_URL` and `PORTAL_URL` in
    `backend/src/apns.rs` stay empty until the official project exists, so a name nobody owns can't receive relay keys
    and push text.

**Test it end to end**

11. Open `PORTAL_URL`: **Sign in with Apple** comes back to "Your keys". Create a key, then revoke it.
12. In a Loft with no `.p8` of its own: Settings → Push notifications → **Sign in** → Allow on the portal. Settings then
    says "Using the Loft relay". Press **Send test push** next to a phone.
13. On the portal, **Delete my account** (sign in again to confirm): the Loft's next push fails with `401`, and
    appleid.apple.com → Sign in with Apple no longer lists the portal.

## Running without the portal

Leave the portal's settings unset. The relay works as before, `/` says "The push portal isn't configured", and
keys come only from the operator's script (below). In Loft, set only the Relay address and paste the key.

## Runbook: make a relay key

With the store's credentials in your shell (Vercel → Storage → the store → `.env.local` tab):

```sh
cd relay
KV_REST_API_URL=… KV_REST_API_TOKEN=… node scripts/create-key.js "Sam (sam@example.com)"
```

It prints the key once; only its hash is stored, with the label and the date. Send the key to that person privately.
They paste it in Loft → Settings → Push notifications.

## Runbook: revoke a relay key

A key made on the portal is revoked there by its owner. For one made with the script, the store holds `relay-key:<sha256 of the key>` with its label, never the key itself. To stop one:

1. **Find it.** In the store's Data Browser, search `relay-key:*` and open them: the value has the label it was made
   for. Or, from the key itself: `printf %s 'lrk_…' | shasum -a 256`. For a busy one, the counters
   `limit:pushesPerDay:<hash>:<day>` name its hash.
2. **Delete** `relay-key:<hash>` in the Data Browser (or `DEL relay-key:<hash>` in its CLI).
3. From then on that Loft gets `401` and its pushes stop. To let that person back in, make them a new key.

To stop the whole relay, remove the project's `APNS_KEY_P8`, or revoke the key in the Apple Developer portal.

## Test

`npm test` runs the tests with Node's own test runner (Node 22), against a fake Apple server and a fake store. No
Apple key is needed.
