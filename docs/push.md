# Push notifications (APNs)

Alerts reach the iPhone app when it's closed. The backend sends them through Apple's Push Notification
service, by one of two paths (ADR 0002):

| | **Your own key** | **The Loft relay** |
|---|---|---|
| When | A `.p8` key, key id and team id in Settings | A relay key in Settings, and no key of your own |
| Setup | The steps below (an Apple developer account) | Paste the relay key you were given ("Sign in to get push") |
| Path | Loft → Apple | Loft → push relay → Apple |
| Who sees the push text | Only Apple | The relay, in transit only (never stored or logged) |
| Apps | Any bundle id (your own build) | The official Loft app only |

Your own key wins when both are set. A relay key only counts when Loft knows the relay's address: Settings → Push
notifications → Relay address, else `LOFT_RELAY_URL`, else `RELAY_URL` in the code (empty until the operator's relay is
deployed). The saved address wins and takes effect at once. With neither, **nothing is pushed**: items that come in meanwhile aren't sent
late once one is added. Settings → Push notifications shows which ("Your own key", "Using the Loft relay" or "Push is
off"), and `GET /api/push` reports it as `path: "own_key" | "relay" | "none"`. Everything else is the same on both
paths: which phones, which kinds, quiet hours, the rate limit and the payload.

## The Loft relay

The relay's and portal's addresses are push settings: https only, no user name or password in them, and set only
through `PUT /api/push` (Settings), never through the general settings or an agent tool. A saved address wins over
`LOFT_RELAY_URL` / `LOFT_PORTAL_URL`, which stay as a fallback for scripted installs.

The relay is login-only. With the relay and portal addresses set, Settings → Push notifications → **Sign in** asks the
relay for a one-time code (10 minutes, at most 5 per IP per hour) and opens the portal with it. After signing in with Apple and pressing
Allow there, this backend, polling the relay, receives a new relay key once and stores it: the key never passes
through the browser, and the code stops working. Approving from a phone works too. Until the portal is live the operator makes
keys with `relay/scripts/create-key.js`. When the portal's address is known (Settings → Portal address, else `LOFT_PORTAL_URL`, else `PORTAL_URL`
in `backend/src/apns.rs`, empty until it's live), Settings shows **Sign in** and, with a key stored, "Manage keys"; without it, only the text. Pasting a key in Settings → Push notifications → Loft relay stays as a fallback. Either way it is kept owner-only in
the same folder as the `.p8` (`relay-key`): never logged, returned by the API or shown to agents. Each push sends the
phone's token, its environment, the bundle id and the payload; the relay forwards them to Apple and passes Apple's
answer back, so a gone phone is removed as it is with a key. If the relay can't be reached, or refuses a push (its
limits are 20 a minute and 1,000 a day per key), the push is logged and dropped, and the next one is tried. A revoked
key gets `401`: pushes stop until a new key is pasted. The relay itself, its API, deploying it, and making and
revoking keys are in [`relay/README.md`](../relay/README.md).

## Setting up your own key (by hand, once)

1. **App id:** developer.apple.com → Certificates, IDs & Profiles → Identifiers → your app id (`LOFT_BUNDLE_ID`) → tick **Push Notifications** → Save. (The app project adds the matching *Push Notifications* capability and the `aps-environment` entitlement.)
2. **Auth key:** same portal → **Keys** → **+** → name it "Loft push", tick **Apple Push Notifications service (APNs)** → Continue → Register.
3. **Download** `AuthKey_XXXXXXXXXX.p8`. Apple lets you download it **only once**: keep a copy in your password manager. Note the **Key ID** (10 characters).
4. **Team ID:** top right of the portal, or Membership details.
5. **Loft → Settings → Push notifications:** choose the `.p8` file (the key id is filled from its name), enter the Team ID, Save.
6. Install a build with push enabled, allow notifications when asked, then press **Send test push** next to the phone in Settings.

## Where the key lives

`~/Library/Application Support/Loft/apns/AuthKey.p8`, file mode `0600` in a `0700` folder. The relay key, when
one is set, is `relay-key` in the same folder, with the same modes. That's outside the repo
and outside Loft's data folder (`.loft/`), so the backup plan (#79) only includes it encrypted.

- It's checked to be a P-256 private key before it's stored.
- It's never logged, returned by the API (`GET /api/push` only says whether one is present) or shown to agents.
- Settings → Push notifications can replace it; `PUT /api/push {remove_key: true}` deletes it.
- The provider token (JWT, ES256) is cached for 45 minutes. Apple wants it refreshed within an hour and no more than every 20 minutes.

## Sandbox vs production (TestFlight)

| Build | APNs environment | Host |
|---|---|---|
| Run from Xcode (debug) | `development` | api.sandbox.push.apple.com |
| **TestFlight** | `production` | api.push.apple.com |
| App Store | `production` | api.push.apple.com |

Each device sends its environment when it registers (the app knows from its `aps-environment` entitlement), and the server
uses the right host per device. The same auth key works for both.

## API

| | |
|---|---|
| `POST /api/devices` | `{token (hex), platform: "ios", environment: "development"\|"production", bundle_id, name?, kinds?, quiet_hours?}`. An upsert by token: call it on every launch. A refresh without `kinds` / `quiet_hours` keeps the earlier choice. Returns the device. |
| `GET /api/devices` | The registered phones. Refused to other websites. |
| `PATCH /api/devices/{token}` | `{kinds?, quiet_hours? (null clears), name?}` |
| `DELETE /api/devices/{token}` | Stop pushing to this phone. |
| `POST /api/devices/{token}/test` | Sends a test push. 410 if Apple says the token is gone (the device is then removed). |
| `GET /api/push` | `{path: "own_key"\|"relay"\|"none", relay_key (whether one is stored), relay_url and portal_url (in use), relay_address and portal_address (saved), configured, key_present, key_id, team_id, bundle_id, devices}`. Never the key or the relay key. |
| `PUT /api/push` | `{key_id?, team_id?, bundle_id?, key_p8?, remove_key?, relay_key?, remove_relay_key?, relay_url?, portal_url?}` |

`kinds` are "needs you" item types (see `docs/needs-you-api.md`): `permission`, `task_approval`, `task_blocked`, `question`,
`grill_round`, `stuck`, `task_failed`, `pr_ready`, `pr_review`. The default is all but `task_failed`, `pr_ready` and `pr_review`.
`grill_question`, the name before rounds, is still accepted and stored as `grill_round`.
`quiet_hours`: `{start: "22:00", end: "07:00", tz: "Europe/Lisbon", utc_offset_minutes: 60}`.

When Apple answers **410**, or **400 BadDeviceToken / DeviceTokenNotForTopic**, the device is removed.

## Payload

```json
{
  "aps": {
    "alert": { "title": "Web Dev needs your approval", "body": "Bash" },
    "sound": "default",
    "badge": 3,
    "thread-id": "<agent id>",
    "interruption-level": "time-sensitive"
  },
  "needs_you_id": "permission:…", "type": "permission", "urgency": 3,
  "agent_id": "…", "task_id": null,
  "target": { "screen": "chat", "agent_id": "…" }
}
```

- **Names and titles only:** never tool input, message text, tokens or secrets.
- `badge` is the "needs you" count.
- `thread-id` groups a phone's notifications by agent. `apns-collapse-id` is the item id (hashed if longer than 64 bytes).
- `time-sensitive` is used for urgency 3 (needs the *Time Sensitive Notifications* capability in the app); everything else is `active`.
- `target` has the same shape as a "needs you" `open` action.
- Test pushes carry `type: "test"`.
- No silent or background pushes.

## What triggers a push

Every 5 seconds the "needs you" watcher hands the current list to the pusher (`push.rs`):

- **Only new items.** An item is pushed at most once (its id is remembered for 30 days). A changed item gets a new id (`docs/needs-you-api.md`). Dismissed or resolved items leave the list, so they're never pushed again.
- **Nothing old.** Items already waiting when Loft starts, or that came in while push wasn't set up, are marked as seen and not pushed late.
- **Per device:** only the `kinds` it chose (by default all but `task_failed` and `pr_ready`), and nothing during its quiet hours (the item stays in the "needs you" list).
- **Rate limit:** at most 5 pushes per device per 10 minutes. The rest become one "N more things need you" summary per window (`type: "summary"`, `target: {screen: "needs_you"}`).

| type | title | body |
|---|---|---|
| permission | "<agent> needs your approval" | "Wants to use <tool name>" |
| task_approval | "<agent> needs your approval" | issue title |
| task_blocked | "<agent> is blocked" | issue title |
| question | "<agent> asked you something" | "Open the chat to answer" |
| grill_round | "<agent>: round <n>" | "Interview: <topic>" |
| stuck | "<agent> may be stuck" | "No activity for N min" / "Waiting on a background task" |
| task_failed | "An issue failed" | issue title |
| pr_ready | "Ready to merge" | "<repo>#<n>: <title>" |
| pr_review | "Review requested" | "<repo>#<n>: <title>" |

Push text is built from names and titles only, never from the item's `reason` or `text`: no tool input, no message content, no errors.
It's still run through the same credential redaction. Send errors are logged with the device name and status, never the payload.
