import LoftKit
import SwiftUI

/// Above the message box while Talk's tidied text waits in it: a Send button whose ring fills for a few seconds, then
/// "Send it?" with Send and Edit. Nothing goes by itself. With the mic open (shown), "send it" sends and "edit" or
/// "cancel" stop it; matching is local, on the words alone. Typing in the box ends it.
struct TalkConfirmBar: View {
    @Environment(LoftStore.self) private var store
    @AppStorage(Prefs.talkAskToSend) private var askAfterPause = true
    @AppStorage(Prefs.talkSayToSend) private var sayToSend = true
    let text: String
    /// What the box holds now: typing in it ends the ask.
    let current: String
    let send: () -> Void
    let cancel: () -> Void
    let done: () -> Void
    @State private var state: TalkConfirm.State
    @State private var speech = SpeechTranscriber()
    @Environment(\.scenePhase) private var scenePhase
    /// When the mic opened, for the cap on how long it listens.
    @State private var since = Date()
    @State private var now = Date()

    private var listening: Bool {
        TalkConfirm.listens(state, sayToSend: sayToSend, active: scenePhase == .active, listenedFor: now.timeIntervalSince(since))
    }

    init(text: String, current: String, startAsking: Bool = false, send: @escaping () -> Void, cancel: @escaping () -> Void, done: @escaping () -> Void) {
        self.text = text
        self.current = current
        self.send = send
        self.cancel = cancel
        self.done = done
        _state = State(initialValue: startAsking ? .asking : .counting(since: Date()))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                if state == .asking {
                    Text("Send it?").font(.headline).accessibilityIdentifier("talkAsk")
                } else {
                    Text("Read it over, then send.").font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Button("Edit") { apply(.touched) }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("talkEdit")
                Button { apply(.sendTapped) } label: { sendLabel }
                    .buttonStyle(.accent)
                    .disabled(!store.canWrite)
                    .accessibilityIdentifier("talkSend")
            }
            .controlSize(.small)
            if listening { micLine }
        }
        .padding(.horizontal, Theme.gutter).padding(.vertical, 8)
        .background(.bar)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("talkConfirm")
        .sensoryFeedback(.impact(weight: .light), trigger: state == .asking)
        .task {
            // Four times a second: the ring, the ask once the pause is long enough, and the mic's cap.
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(250))
                now = Date()
                if case .counting = state { apply(.tick(now)) }
            }
        }
        // In the background, or past the cap, the mic closes.
        .task(id: listening) {
            if listening { await speech.start() } else { speech.stop() }
        }
        .onChange(of: speech.transcript) { _, heard in
            let names = store.agents.map(\.name)
            if let c = TalkConfirm.command(in: heard, agents: names, asking: state == .asking) {
                apply(.heard(c))
            } else if heard.split(separator: " ").count > 8 {
                // Talk that isn't a command: start over, so a "send it" said next is heard on its own.
                speech.stop()
                Task { await speech.start() }
            }
        }
        .onChange(of: current) { _, now in if now != text { apply(.touched) } }
        .onDisappear { speech.stop() }
    }

    /// The Send button, with the ring that fills while it waits.
    @ViewBuilder private var sendLabel: some View {
        HStack(spacing: 6) {
            if case .counting(let since) = state, askAfterPause {
                TimelineView(.periodic(from: since, by: 0.25)) { ctx in
                    Circle().trim(from: 0, to: TalkConfirm.progress(since: since, now: ctx.date))
                        .stroke(Theme.onAccent, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                        .background(Circle().stroke(Theme.onAccent.opacity(0.3), lineWidth: 2.5))
                        .frame(width: 14, height: 14)
                }
                .accessibilityHidden(true)
            }
            Text("Send")
        }
    }

    @ViewBuilder private var micLine: some View {
        switch speech.state {
        case .denied(let m), .unavailable(let m), .failed(let m):
            Label("Saying “send it” isn't available: \(m)", systemImage: "mic.slash")
                .font(.caption).foregroundStyle(.secondary)
        default:
            Label("Listening: say “send it”, “edit” or “cancel”", systemImage: "waveform")
                .font(.caption).foregroundStyle(Theme.accent)
                .symbolEffect(.variableColor.iterative, isActive: speech.state.isRecording && !TestMode.noAnimations)
                .accessibilityIdentifier("talkListening")
        }
    }

    private func apply(_ e: TalkConfirm.Event) {
        let next = TalkConfirm.reduce(state, e, askAfterPause: askAfterPause)
        guard next != state else { return }
        state = next
        switch next {
        case .sending: speech.stop(); send()
        case .cancelled: speech.stop(); cancel()
        case .editing: speech.stop(); done()
        default: break
        }
    }
}
