import SwiftUI

/// "While you were away" at the top of the Agents tab: one line per agent that did something worth knowing since
/// you last looked (failed, blocked, waiting for you, finished, opened a PR). Tap a line for that chat.
struct DigestCard: View {
    @Environment(LoftStore.self) private var store
    @Environment(Speaker.self) private var speaker
    @Environment(Router.self) private var router
    /// Set when the app went to the background; "Got it" moves it to now.
    @AppStorage(Prefs.digestLastLooked) private var lastLooked: Double = 0
    /// This card's read-aloud is playing: the button turns into Stop.
    @State private var reading = false

    private var digest: Digest? {
        guard lastLooked > 0 else { return nil }
        let since = Date(timeIntervalSince1970: lastLooked)
        guard Date().timeIntervalSince(since) >= Digest.awayFor || Self.fixture else { return nil }
        let d = Digest.build(since: since, activity: store.activity, pullRequests: store.pullRequests, agents: store.agents)
        return d.lines.isEmpty ? nil : d
    }

    #if DEBUG
    /// `-fixture-digest`: as if you'd been away for two hours.
    static var fixture: Bool { ProcessInfo.processInfo.arguments.contains("-fixture-digest") }
    #else
    static let fixture = false
    #endif

    var body: some View {
        if let d = digest {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Label("While you were away", systemImage: "clock.arrow.circlepath").font(.subheadline.weight(.semibold))
                    Spacer()
                    Text(d.since, format: .relative(presentation: .named)).font(.caption).foregroundStyle(.secondary)
                }
                ForEach(d.lines) { l in
                    Button { router.openChat(l.agentId) } label: {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Circle().fill(l.failed ? Theme.failure : l.needsYou ? Theme.attention : Theme.working).frame(width: 7, height: 7).accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(l.agentName).font(.footnote.weight(.semibold))
                                Text(l.text).font(.footnote).foregroundStyle(.secondary).lineLimit(3)
                            }
                            Spacer(minLength: 0)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityElement(children: .combine)
                    .accessibilityHint("Opens the chat")
                }
                HStack {
                    if reading && speaker.isSpeaking {
                        Button { speaker.stop() } label: { Label("Reading… Stop", systemImage: "stop.fill") }
                            .accessibilityLabel("Stop reading").accessibilityIdentifier("digestReadAloud")
                    } else {
                        Button {
                            reading = true
                            Task { _ = await speaker.speak(spoken(d)); reading = false }
                        } label: { Label("Read aloud", systemImage: "speaker.wave.2") }
                            .accessibilityIdentifier("digestReadAloud")
                    }
                    Spacer()
                    Button("Got it") { lastLooked = Date().timeIntervalSince1970 }.accessibilityIdentifier("digestDismiss")
                }
                .font(.footnote)
                .buttonStyle(.borderless)
            }
            .padding(.vertical, 4)
            .accessibilityIdentifier("digestCard")
        }
    }

    private func spoken(_ d: Digest) -> String {
        "While you were away. " + d.lines.map { "\($0.agentName): \($0.text)." }.joined(separator: " ")
    }
}
