import AppKit
import LoftKit
import SwiftUI

/// Settings › Apple Intelligence on the Mac: the on-device model's status and the iPhone's switches, in its order and
/// words. A switch is off and disabled while the model can't run; the setting is kept.
struct AppleIntelligenceSection: View {
    @AppStorage(Prefs.aiCommands) private var aiCommands = true
    @AppStorage(Prefs.aiShortSpoken) private var aiShortSpoken = true
    @AppStorage(Prefs.aiNeedsYou) private var aiNeedsYou = true
    @AppStorage(Prefs.aiQuickAck) private var aiQuickAck = true
    @AppStorage(Prefs.aiRoute) private var aiRoute = true
    @AppStorage(Prefs.aiChatReplies) private var aiChatReplies = true
    @AppStorage(Prefs.aiExplain) private var aiExplain = true
    /// Re-read every few seconds and on coming back to the app, so it changes as soon as the model is ready.
    @State private var status = OnDeviceModel.status

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Apple Intelligence").font(.headline)
            LabeledContent("On-device model", value: status.text)
                .help("macOS says: \(OnDeviceModel.rawAvailability)")
                .accessibilityIdentifier("onDeviceModelStatus")
            if let hint = status.hint {
                Text(hint).font(.caption).foregroundStyle(.secondary)
                    .accessibilityIdentifier("onDeviceModelHint")
            }
            toggle("Understand voice commands", $aiCommands, id: "aiCommandsToggle",
                   note: "Tells an answer from a command like “skip” when the usual words don't match. It only ever asks.")
            toggle("Short spoken versions", $aiShortSpoken, id: "aiShortSpokenToggle",
                   note: "Reads a long reply as a few plain sentences; say “more” to hear it all.")
            toggle("Summaries in Needs you", $aiNeedsYou, id: "aiNeedsYouToggle",
                   note: "One line per item and suggested replies, which only fill in the reply box.")
            toggle("Quick first reply", $aiQuickAck, id: "aiQuickAckToggle",
                   note: "Says at once what the Orchestrator will look into, until the real reply comes.")
            toggle("Suggest who it's for", $aiRoute, id: "aiRouteToggle",
                   note: "Offers “Send to …?” when a message looks meant for one agent. Nothing goes without your click.")
            toggle("Suggested replies in chats", $aiChatReplies, id: "aiChatRepliesToggle",
                   note: "When an agent asks you something, offers two or three replies. A click fills in the reply; you send it.")
            toggle("Explain permission requests", $aiExplain, id: "aiExplainToggle",
                   note: "“What does this do?” on an approval says it in plain words. The risk labels come from the app, not the model, and you still decide.")
            Text("Runs on this Mac, works offline and sends nothing anywhere. When it's not available, Loft works as before.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .task {
            while !Task.isCancelled {
                status = OnDeviceModel.status
                try? await Task.sleep(for: .seconds(3))
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            status = OnDeviceModel.status
        }
    }

    private func toggle(_ title: String, _ value: Binding<Bool>, id: String, note: String) -> some View {
        let usable = status.isAvailable
        return Toggle(isOn: usable ? value : .constant(false)) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(note).font(.caption).foregroundStyle(.secondary)
            }
        }
        .disabled(!usable)
        .accessibilityIdentifier(id)
    }
}
