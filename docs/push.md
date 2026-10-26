# Push notifications (APNs)

Alerts reach the iPhone app when it's closed. The backend sends them through Apple's Push Notification
service with token-based auth (an APNs auth key, `.p8`).

## Setting it up (by hand, once)

1. **App id:** developer.apple.com → Certificates, IDs & Profiles → Identifiers → your app id (`LOFT_BUNDLE_ID`) → tick **Push Notifications** → Save. (The app project adds the matching *Push Notifications* capability and the `aps-environment` entitlement.)
2. **Auth key:** same portal → **Keys** → **+** → name it "Loft push", tick **Apple Push Notifications service (APNs)** → Continue → Register.
3. **Download** `AuthKey_XXXXXXXXXX.p8`. Apple lets you download it **only once**: keep a copy in your password manager. Note the **Key ID** (10 characters).
4. **Team ID:** top right of the portal, or Membership details.
5. **Loft → Settings → Push notifications:** choose the `.p8` file (the key id is filled from its name), enter the Team ID, Save.
6. Install a build with push enabled, allow notifications when asked, then press **Send test push** next to the phone in Settings.

## Where the key lives

`~/Library/Application Support/Loft/apns/AuthKey.p8`, file mode `0600` in a `0700` folder. That's outside the repo
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
| `GET /api/push` | `{configured, key_present, key_id, team_id, bundle_id, devices}`. Never the key. |
| `PUT /api/push` | `{key_id?, team_id?, bundle_id?, key_p8?, remove_key?}` |

`kinds` are "needs you" item types (see `docs/needs-you-api.md`): `permission`, `task_approval`, `task_blocked`, `question`,
`grill_question`, `stuck`, `task_failed`, `pr_ready`, `pr_review`. The default is all but `task_failed`, `pr_ready` and `pr_review`.
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
| task_approval | "<agent> needs your approval" | task title |
| task_blocked | "<agent> is blocked" | task title |
| question | "<agent> asked you something" | "Open the chat to answer" |
| grill_question | "<agent>: question <n>" | "Interview: <topic>" |
| stuck | "<agent> may be stuck" | "No activity for N min" / "Waiting on a background task" |
| task_failed | "A task failed" | task title |
| pr_ready | "Ready to merge" | "<repo>#<n>: <title>" |
| pr_review | "Review requested" | "<repo>#<n>: <title>" |

Push text is built from names and titles only, never from the item's `reason` or `text`: no tool input, no message content, no errors.
It's still run through the same credential redaction. Send errors are logged with the device name and status, never the payload.
