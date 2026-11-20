import LoftKit
import SwiftUI

/// Shown instead of starting a Delivery whose tickets wait on open tickets in another spec, like the web's
/// StartWarning: says what they wait on and offers to start the other spec first (or to start anyway: the waiting
/// tickets stay waiting until those close).
struct StartWarning: View {
    @Environment(LoftStore.self) private var store
    let waits: [WaitsOn]
    let onStart: () -> Void
    let onCancel: () -> Void
    /// The other spec started: reload what this one waits on.
    let onStarted: () -> Void
    @State private var busy: String?
    @State private var error: String?

    var body: some View {
        let first = WaitsOn.specsToStartFirst(waits)
        VStack(alignment: .leading, spacing: 8) {
            Label {
                Text(Self.text(waits, canStartFirst: !first.isEmpty))
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Theme.attention)
            }
            .font(.footnote)
            .accessibilityIdentifier("startWarning")
            ForEach(first, id: \.self) { f in
                let key = "\(f.repo)#\(f.spec)"
                Button { startFirst(f.issueId, key) } label: {
                    Label {
                        Text("Start \(key) first")
                    } icon: {
                        if busy == key { ProgressView().controlSize(.mini) } else { Image(systemName: "play.fill") }
                    }
                }
                .buttonStyle(.accent)
                .disabled(busy != nil || !store.canWrite)
                .accessibilityIdentifier("startFirst-\(key)")
            }
            HStack(spacing: 16) {
                Button("Start anyway", action: onStart).disabled(busy != nil || !store.canWrite).accessibilityIdentifier("startAnyway")
                // Cancel changes nothing, so it stays on when the connection is read-only.
                Button("Cancel", action: onCancel).disabled(busy != nil).accessibilityIdentifier("startWarningCancel")
            }
            .buttonStyle(.borderless)
            .font(.footnote)
            if let error { Text(error).font(.caption2).foregroundStyle(Theme.failureInk) }
        }
        .padding(.vertical, 2)
    }

    /// "Tickets of this spec wait on octo/api#12, which isn't done yet. …", the web's wording.
    static func text(_ waits: [WaitsOn], canStartFirst: Bool) -> String {
        let blockers = waits.map(\.blocker).joined(separator: ", ")
        let tail = canStartFirst
            ? " Start the spec that builds it first, or start this one and let those tickets wait."
            : " Start anyway and those tickets wait until it closes."
        return "Tickets of this spec wait on \(blockers), which \(waits.count == 1 ? "isn't" : "aren't") done yet.\(tail)"
    }

    private func startFirst(_ issueId: String, _ key: String) {
        busy = key
        error = nil
        Task {
            do {
                try await store.deliveryAction(issueId, .start)
                onStarted()
            } catch {
                self.error = APIError.from(error).localizedDescription
            }
            busy = nil
        }
    }
}
