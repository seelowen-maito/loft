# Native Mac app: proposal and TODO

**Status:** proposal, waiting for a decision. Nothing is built yet.

## Why

Loft on the Mac is a web page today. The iOS app (SwiftUI, about 8,400 lines) already covers agents, chat (with grill mode), tasks, voice and notifications. Most of that can be reused, so a native Mac app is mostly new screens, not a rewrite.

A Mac app can also do something the web page can't: own the backend. It could show a menu-bar status, start and restart it, launch it at login, or bundle it, so Loft becomes one app you install instead of a script you run.

## What can be reused

| Part of the iOS app | Lines | On the Mac |
|---|---|---|
| `Core/`: API client, live events, cache, voice engine | ~2,100 | as is, except the iOS audio session in voice (needs a Mac path) |
| `Domain/`: models, grill and task logic | ~770 | as is |
| `App/`: the store that drives every screen | ~880 | as is |
| `DesignSystem/`: stickers, chips, colours, type | ~1,100 | as is, minor tweaks |
| `Features/`: the screens | ~3,500 | partly: phone layouts need Mac versions |

iOS-only API calls are few: 9 × `navigationBarTitleDisplayMode`, `AVAudioSession` (voice), and one each of `UIPasteboard`, `UIDevice` and `UIApplication`. All can be wrapped in `#if os(iOS)`. Speech recognition (`SFSpeechRecognizer`) exists on macOS.

## Options

1. **Run the iPhone app on Apple Silicon Macs.** Almost no work: tick the option in App Store Connect. It's a phone-sized window, with nothing from the web app that the iOS app lacks.
2. **Mac Catalyst.** About 1 day. A resizable window from the same code, but it feels like an iPad app.
3. **Native macOS target in the same Xcode project (recommended).** Shares `Core`, `Domain`, `App` and `DesignSystem`, with new Mac screens: a sidebar of agents by project, chat in the middle, an inspector on the right (like the web), menu commands and keyboard shortcuts.

## TODO (if option 3 is chosen)

**Milestone 1: first usable version (about 3–4 days of agent work)**
- [ ] Add a `Loft macOS` target sharing `Core`, `Domain`, `App` and `DesignSystem`; wrap iOS-only calls in `#if os(iOS)`
- [ ] Three-column window: projects and agents sidebar, chat, inspector
- [ ] Chat at parity with iOS, including grill question cards, quick actions and the summary
- [ ] Tasks list and detail
- [ ] Voice: a Mac audio path for recording and speaking
- [ ] Notifications (permission requests, finished runs, grill questions)
- [ ] Menu bar commands and shortcuts (⌘N new agent, ⌘K switch agent, ⌘. stop)
- [ ] UI tests for the main flows, run by Tester

**Milestone 2: parity with the web app (about 1–2 weeks)**
- [ ] Agent setup: inspector fields, new agent, quiet hours, permissions
- [ ] Repos page
- [ ] Settings (projects folder, GitHub token, iMessage, voice)
- [ ] Agent activity and usage charts
- [ ] Live Simulator viewer (stream and input, as on `/simulator`)
- [ ] Update banner and status

**Milestone 3: the app owns the backend (optional)**
- [ ] Menu-bar extra: backend status, start / restart / stop, open logs
- [ ] Launch at login
- [ ] Bundle the backend binary and the built web UI; first-run setup
- [ ] Signing, notarization and an update channel (Sparkle or the Mac App Store)

## Open questions

1. Go with option 3 (native target), the cheap options 1 or 2, or not now?
2. Should the Mac app manage the backend (milestone 3), or stay a client like the iOS app?
3. Distribution: Mac App Store, notarized download, or TestFlight for Mac?
4. Minimum macOS version: 14 (Sonoma) matches the iOS 17 APIs already in use.
