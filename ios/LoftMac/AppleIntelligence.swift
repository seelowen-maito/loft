import LoftKit
import SwiftUI

/// Settings › Apple Intelligence on the Mac (spec #382): the on-device model's status and the iPhone's switches for
/// the features this app has. A switch is off and disabled while the model can't run; the setting is kept.
struct AppleIntelligenceSection: View {
    @AppStorage(Prefs.aiRoute) private var aiRoute = true
    @AppStorage(Prefs.aiChatReplies) private var aiChatReplies = true
    @AppStorage(Prefs.aiNeedsYou) private var aiNeedsYou = true
    @State private var status = OnDeviceModel.status

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Apple Intelligence").font(.headline)
            LabeledContent("On-device model", value: status.text)
                .accessibilityIdentifier("onDeviceModelStatus")
            toggle("Suggest who it's for", $aiRoute, id: "aiRouteToggle",
                   note: "Offers “Send to …?” when a message looks meant for one agent. Nothing goes without your click.")
            toggle("Suggested replies in chats", $aiChatReplies, id: "aiChatRepliesToggle",
                   note: "When an agent asks you something, offers two or three replies. A click fills in the reply; you send it.")
            toggle("Summaries in Needs you", $aiNeedsYou, id: "aiNeedsYouToggle",
                   note: "One line per item and suggested replies, which only fill in the reply box.")
            Text("Runs on this Mac, works offline and sends nothing anywhere. When it's not available, Loft works as before.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .task {
            while !Task.isCancelled {
                status = OnDeviceModel.status
                try? await Task.sleep(for: .seconds(3))
            }
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
