# Loft for iOS

A native SwiftUI client for Loft. It shows what your agents are doing, lets you talk to them, and lets you track their work as tasks from your iPhone. Loft itself keeps running on the Mac. The app is only a remote control: closing it or losing signal doesn't stop any agent.

## How it fits Loft

What was already there, and what the app uses:

| Loft (backend/, Rust + axum + SQLite) | Used by the app for |
| --- | --- |
| `GET /api/agents`, `GET /api/agents/{id}` | Agent list, detail, status (`idle`, `running`, `waiting`, `error`) |
| `GET/POST /api/agents/{id}/messages` | Conversations (one per agent, paged with `before`) |
| `POST /api/agents/{id}/run`, `/stop` | "Wake up now", "Stop current run" |
| `GET /api/permissions`, `POST /api/permissions/{id}` | Approvals (allow once / always / deny) |
| `GET /api/pull-requests` | PRs on the agent screen |
| `GET /api/events` (server-sent events) | Live updates |
| `GET /api/info` | Connection test, version, feature detection |

Agents are headless Claude Code sessions. They already message each other through the `message_agent` MCP tool, which is how one agent delegates to another. Loft has no built-in orchestrator, so the app lets you **pick the agent that acts as one** (Settings → Orchestrator, or the Chat tab). If an agent's name contains "orchestrator", the app picks it for you. Messages to that agent go to that agent and nowhere else. The app has no planning logic of its own.

### What this change adds to the backend

The smallest additions that make delegated work visible. The source of truth stays in Loft's database:

- **Tasks** (`backend/src/tasks.rs`, a `tasks` table): personal todos and work handed to agents, with parent/child links and dependencies.
  - You own the title, description, priority, project, assignee and due date.
  - The agent working on a task owns its execution status, which it reports with new MCP tools: `list_tasks`, `create_task` (for delegated subtasks, optionally started straight away) and `update_task`.
  - When a delegated subtask is done, failed or blocked, Loft messages the agent that delegated it.
- **Activity feed** (`GET /api/activity`): runs, failures, approvals, agent-to-agent messages and task milestones. It is built from stored history, so it also covers what happened while the phone was away.
- **Idempotent messages**: `POST /api/agents/{id}/messages` accepts a `client_id`. Resending with the same id returns the original message instead of delivering it twice. The response now includes the stored message.
- **`GET /api/info`** reports `features` and the tailnet `host`. When it talks to an older Loft, the app hides tasks and activity and says why, instead of failing.
- **SSE events** `task` and `task_deleted`.
- The agent system prompt has one new line that tells agents about the task tools.

## API added

All endpoints return JSON. Errors are `{ "error": "message" }` with a 400, 404 or 409 status. Timestamps are RFC 3339 in UTC.

**Task object**

```json
{
  "id": "uuid", "title": "…", "description": "…",
  "kind": "todo | work",
  "status": "inbox | ready | in_progress | blocked | waiting_approval | done | failed | cancelled",
  "priority": "low | normal | high | urgent",
  "project": "", "agent_id": null, "parent_id": null, "depends_on": [], "due_at": null,
  "created_by": "user | <agent id>",
  "note": "latest progress", "summary": "", "error": "",
  "created_at": "…", "updated_at": "…", "started_at": null, "completed_at": null
}
```

| Method & path | Body | Notes |
| --- | --- | --- |
| `GET /api/tasks` | — | Optional filters: `status` (comma-separated), `agent_id`, `parent_id`, `project`. |
| `POST /api/tasks` | `title` (required), `description`, `priority`, `project`, `agent_id`, `parent_id`, `depends_on`, `due_at`, `kind`, `status` (`inbox` or `ready`), `id` | `id` is a client-chosen UUID: sending it again returns the existing task (200) instead of creating another (201). A new task never runs by itself. |
| `GET /api/tasks/{id}` | — | |
| `PATCH /api/tasks/{id}` | Any of the fields you own, plus `status` set to `inbox`, `ready` or `done` | 409 while an agent is working on it (cancel first). Parent cycles and self-dependencies are rejected. |
| `DELETE /api/tasks/{id}` | — | 409 while active. Subtasks and dependents are kept and their links cleared. |
| `POST /api/tasks/{id}/execute` | `agent_id` (optional if assigned), `note` | Hands the task to the agent as a message and starts it, or queues it behind the agent's current run. Works from inbox, ready or failed (that is a retry). 409 when it's already running or closed. Never retried automatically. |
| `POST /api/tasks/{id}/cancel` | `stop_run` (default `false`) | Marks the task cancelled. A busy agent is told to drop it. The agent's run is only stopped if `stop_run` is true, because that also ends anything else the run is doing. |
| `GET /api/activity` | — | `limit` (default 50, max 200) and `before` (RFC 3339) for paging. Items: `id, type, at, agent_id, task_id, message_id, summary, severity`. |

Agent MCP tools (at `/mcp/{agent_id}`, as before): `list_tasks`, `create_task` and `update_task`. An agent can only update tasks assigned to it or created by it, and can't move a cancelled task.

There is no shell or command endpoint. The app can only use what's listed above.

## The app

```
ios/
  Loft.xcodeproj         Xcode 16+ project (folders sync automatically, no dependencies)
  Config/Info.plist          ATS: local networking only (see Security)
  Loft/
    App/                     LoftApp (tabs, routing), LoftStore (all client state)
    Core/                    LoftAPI (URLSession), realtime (SSE parser, backoff), errors,
                             snapshot cache, notifications, UI-test fixtures (Debug only)
    Domain/                  Codable models, status mapping, JSON helpers
    DesignSystem/            Colors, status badges, cards, empty states
    Features/                Overview, Agents, Tasks, Chat (incl. dictation), Settings
  LoftTests/             Unit tests
  LoftUITests/           UI tests against fixtures
```

- **One store, server-authoritative.** `LoftStore` (`@MainActor @Observable`) holds what the server last said. Every action waits for the server's answer before the UI changes. Statuses come from the server and the app doesn't invent any. Agent states are normalized to Working, Needs approval, Idle, Last run failed and Unknown, because those are the only states Loft can report. There is no pause, and no blocked state for agents (tasks can be blocked).
- **Realtime.** There is one shared SSE connection. Every time it opens, the app loads a fresh snapshot, because the stream can't replay what was missed. Reconnects use exponential backoff with jitter (1–10 s), and the app retries at once when the network comes back (for example when Tailscale reconnects). If only the stream is blocked, the app falls back to polling. The stream is dropped in the background and reopened in the foreground.
- **Messages.** A sent message shows as *Sending…* until the server confirms it. A failed send stays on screen with Retry and Delete buttons. Retry reuses the same `client_id`, so it is never delivered twice, and the SSE echo is de-duplicated by message id. Tool calls collapse into one line per run. Raw tool input and output are never shown.
- **Offline.** The last snapshot, including recent conversations and unsent messages, is cached on the device. It is always labelled stale with the time it was saved. Every write is disabled while disconnected, so nothing queued runs by surprise later.
- **Dictation.** The microphone uses the Speech framework and prefers on-device recognition. With "On-device only" turned on, audio never leaves the phone. The transcript goes into the text box and is **never sent automatically**, and recording stops completely when you tap stop. Keyboard dictation works too.
- **Notifications.** Local notifications cover approvals, task done/failed/blocked and run failures, and you can switch each kind off. They only fire while the app is running or recently in use (see Limitations).

## Voice conversations

Long dictation, cleaned up on the Mac, with replies read aloud. The engine is in `Loft/Core/Voice`:

- **`VoiceSession`** listens for as long as you talk.
  - On iOS 26 it uses `SpeechAnalyzer`, which is built for long audio.
  - On iOS 17–18 it chains `SFSpeechRecognizer` requests, because Apple caps each one at about a minute and ends it after a long pause. The mic tap never stops. Each request rolls over at the first short pause after 40 s, or at 55 s if you never pause, so no word is cut in half. Each finished segment is appended to the draft, and the next one carries on.
  - Recognition is always on-device in long sessions.
  - Saying **"I'm done"**, "that's it", "terminei" or "pronto" at the end finishes the dictation, and so does the Done button.
- **Cleanup:** the whole draft goes to **`POST /api/voice/clean`**. Loft runs Claude Haiku with no tools to fix punctuation, joins, filler and misheard words. It keeps each part in the language it was spoken in and never follows instructions inside the dictation. If cleanup fails or times out, the original text comes back with `cleaned: false`.
- **Review:** you see the cleaned text, can compare it with the original and edit it. Nothing is sent until you tap Send, or say "send" / "envia" in hands-free mode; only an exact short command counts.
- **`Speaker`** reads the agent's reply aloud with the best installed Apple voice for the reply's language. It skips code, links, tables and Markdown, and stops long replies at a paragraph with "the rest is in the chat".
- **Hands-free** mode listens again after the reply. Background audio (`UIBackgroundModes: audio`) keeps it working with the screen locked, and the live connection stays open while a session is active.
- The draft is saved after every segment, so an unsent dictation survives a crash (`VoiceSession.savedDraft()`).
- **UI fixtures:** debug builds accept `-ui-testing -fixture-voice preparing|listening|paused|cleaning|review|sending|speaking|failed|failed-startup`.

## Setup

### On the Mac

1. Run Loft (`./start.sh` or the background service from `./setup.sh`). It listens on `127.0.0.1:7420`.
2. Expose it **to your tailnet only**: `tailscale serve --bg 7420`. Enable HTTPS certificates in the Tailscale admin console first. `tailscale serve status` shows the address, e.g. `https://my-mac.tailnet-name.ts.net`. Don't use `tailscale funnel`, which makes it public.

### On the iPhone

1. Install the Tailscale app, sign in to the same tailnet, and turn the VPN on. Having Tailscale on the Mac alone doesn't let the phone reach it.
2. Open Loft, enter the `https://….ts.net` address, tap **Test connection**, then **Save and connect**. You can change the address later in Settings.

### Security

- **No app login, by design.** The tailnet is the boundary: only your devices can reach the server, and `tailscale serve` adds valid TLS. Anyone on your tailnet who can reach the Mac can control Loft, the same as with the web UI. Use Tailscale ACLs to limit which devices can reach it. If you share the tailnet, add app-level auth before using this.
- **TLS is always validated.** The app doesn't add ATS exceptions. The only setting, `NSAllowsLocalNetworking`, allows plain HTTP to `localhost` and `.local` for a development backend, and the connect screen warns if you enter `http://` for anything else.
- The app stores no secrets. The server address and preferences are kept in UserDefaults, and the cache is in the app's Caches folder with file protection. Logs never include message text.
- Destructive actions ask for confirmation. Stopping a run (which kills the Claude process) is a separate action from cancelling a task (which marks it cancelled and tells the agent). Approvals use Loft's existing permission system and can't bypass it.

## Running it

**Signing**

The team and bundle id aren't in the repo. Set them in `ios/Config/Local.xcconfig` (gitignored; copy `Local.xcconfig.example`), or as environment variables for `xcodebuild` and the scripts:

```sh
LOFT_TEAM_ID=ABCDE12345          # your Apple Developer team id: needed for a device, an archive or TestFlight
LOFT_BUNDLE_ID=com.you.loft  # default com.example.loft, enough for the Simulator
```

Set the same `LOFT_BUNDLE_ID` (and `LOFT_TEAM_ID`) for the backend to use as push defaults, or enter them in Settings → Push notifications.

**Simulator**

```sh
cd ios
xcodebuild -scheme Loft -destination 'generic/platform=iOS Simulator' -derivedDataPath build build
xcrun simctl install booted build/Debug-iphonesimulator/Loft.app
xcrun simctl launch booted com.example.loft   # or your LOFT_BUNDLE_ID
```

The Simulator uses the Mac's network, so the `.ts.net` address works directly. To point a Debug build at a development backend without saving it, pass `-serverURL-override http://127.0.0.1:7430` to `simctl launch`.

**A physical iPhone:** open `ios/Loft.xcodeproj` in Xcode, set your team and bundle id (see **Signing** below), connect the phone and press Run. A free Apple ID works for personal installs, but the app has to be reinstalled every 7 days.

**TestFlight**

One-time setup:

1. Sign in to Xcode with the Apple Developer Program account (Xcode → Settings → Accounts).
2. In [App Store Connect](https://appstoreconnect.apple.com), create the app: Apps → + → New App, with your bundle id.

Then, for each build, from the repo root:

```sh
./testflight.sh            # tests, archive, upload (about 4 minutes)  
./testflight.sh --fast     # skip the tests (under a minute)
./testflight.sh --no-upload  # archive and export an .ipa to ios/build/testflight/export
VERSION=0.2.0 ./testflight.sh  # also set the version TestFlight shows
```

It does what Product → Archive → Distribute App does in Xcode: archives a Release build with a UTC timestamp as the build number (so every upload is new), signs it with your team and the account signed in to Xcode, and uploads it. `-allowProvisioningUpdates` creates the distribution certificate and profile if they're missing. Output goes to `ios/build/testflight/xcodebuild.log`; on failure the script prints the errors from it. A macOS notification says when the upload is done. For CI, set `ASC_KEY_PATH`, `ASC_KEY_ID` and `ASC_ISSUER_ID` to an App Store Connect API key instead of signing in.

When processing finishes (5–15 minutes), add testers under TestFlight → Internal Testing. Use internal testers only: the app reaches Loft over your tailnet, which Beta App Review (required for external testers) can't reach. Each tester also needs Tailscale on the phone.

The build settings set `ITSAppUsesNonExemptEncryption = NO` (the app only uses HTTPS), so uploads skip the export-compliance question. `Loft/Resources/PrivacyInfo.xcprivacy` declares no tracking, no collected data and the UserDefaults access.

**Tests**

```sh
ios/scripts/test.sh                 # build once, unit tests, then UI tests, with every test's time
ios/scripts/test.sh --unit          # unit tests only: under a minute
ios/scripts/test.sh --no-build --only LoftTests/StoreTests   # rerun one class on the last build
ios/scripts/test.sh --no-build --ui --only LoftUITests/LoftUITests/testOverview   # one UI test
cd backend && cargo test
```

The script builds once (`build-for-testing`), runs the unit tests, and runs the UI tests only if those pass. It prints each test's time, slowest first, and flags a suite that goes over its target: 1 minute for unit tests, 2 for UI tests. Each test gets 60 s, which is the shortest limit Xcode allows. Tests run one at a time on one Simulator, never on clones. Set `SIMULATOR` to pick an already-booted device (the default is the first booted iPhone). The script never creates, boots or erases a Simulator: whoever schedules test runs owns them. Set `DERIVED_DATA` to keep builds warm, and `RESULT` to get result bundles. When `xcode-select` points at the Command Line Tools, the script finds Xcode itself, or you can set `DEVELOPER_DIR`.

**Rerunning without a rebuild.** A full run builds once into `DERIVED_DATA` (default `ios/build`). As long as no code changed since, `--no-build` reuses that build and starts testing straight away, so a failing or slow test can be rerun on its own in seconds with `--only`. Rebuild (drop `--no-build`) after pulling or switching branches, or the run tests the old code. Keep the same `DERIVED_DATA` and Simulator for both runs.

Rules belong in unit tests. UI tests check only that screens show and respond: each one is a single launch walking a whole journey, with `-no-animations -no-speech -no-ai` (no animations, microphone, speech recognizer or on-device model), and waits of 5 s or less.

UI tests run against an in-app fixture server (`-ui-testing`, Debug builds only), so no agents run. The backend tests use a temporary database.

## Limitations and what needs outside setup

- **Push notifications (APNs)** need outside setup: the Push Notifications capability on the app id, and an APNs auth key (`.p8`) added in Loft → Settings → Push notifications. The backend side (device registration at `/api/devices`, the APNs sender, a test push) is in place; see `docs/push.md` for the steps and the payload. Until the app registers for push, alerts only arrive while it's running. Loft's **iMessage notifications** also reach your phone when the app is closed.
- **No pause or resume.** Claude Code runs can't be paused, so the app doesn't offer it. "Stop current run" ends the process.
- **One conversation per agent.** Loft keeps a single history per agent, so there are no separate threads.
- **There's no separate plan-approval step.** The agent's permission mode, shown on its screen, decides what needs your approval. Those requests show up in the app like any other, and the app adds no approval rules of its own.
- **Avatars are monograms.** Loft avatars are SVG, which SwiftUI can't draw natively.
- **Editing a task doesn't re-brief the agent.** Changing a running task's description doesn't change what the agent was already told. Message the agent instead (the app says this when you edit).
