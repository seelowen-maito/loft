import SwiftUI

/// The Delivery card on a spec's or standalone ticket's issue page, like the web's: Start delivery until it
/// started, then who has which ticket on which PR, where it stands (running, stalled and why, paused, needs you,
/// final review or done), the tickets it skips, and Pause or Resume when they apply. It stays live from the
/// server's `delivery` events.
struct DeliveryCardSection: View {
    @Environment(LoftStore.self) private var store
    let task: HBTask
    @State private var busy: IssueDeliveryAction?
    @State private var said: (ok: Bool, text: String)?

    var body: some View {
        let d = store.deliveries[task.id]
        let number = task.github?.number ?? 0
        Section {
            VStack(alignment: .leading, spacing: 4) {
                Text(intro(d, number)).font(.subheadline).foregroundStyle(.secondary)
                if let line = d?.line {
                    Text(line.text)
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(color(line.tone))
                        .accessibilityIdentifier("deliveryState")
                }
            }
            .padding(.vertical, 2)
            if let skipped = d?.skippedTickets, !skipped.isEmpty {
                ForEach(skipped, id: \.number) { x in
                    Text("Skipped #\(x.number) \(x.title): it \(x.skipped), so it's being built elsewhere. It counts as done when it closes.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
            if d?.started != true {
                button(.start, "Start delivery", "Starting delivery…", symbol: "play.fill")
            }
            if d?.showsPause == true {
                button(.pause, "Pause", "Pausing…", symbol: "pause.fill")
            }
            if d?.showsResume == true {
                button(.resume, "Resume", "Resuming…", symbol: "play.fill")
            }
            if let said {
                Text(said.text).font(.footnote).foregroundStyle(said.ok ? Color.secondary : Theme.failureInk)
                    .accessibilityIdentifier("deliverySaid")
            }
        } header: {
            Text("Delivery")
        }
        .task(id: task.id) { try? await store.loadDelivery(task.id) }
        .onAppear { store.showDeliveryCard(task.id) }
        .onDisappear { store.hideDeliveryCard(task.id) }
        .onChange(of: task.id) { old, new in
            store.hideDeliveryCard(old)
            store.showDeliveryCard(new)
        }
    }

    /// What it does before it starts; who has it once it did.
    private func intro(_ d: IssueDelivery?, _ n: Int) -> String {
        if let d, d.started { return d.progressLine(issue: n) }
        return task.isDeliverableSpec
            ? "Loft builds the open sub-issues of #\(n) one at a time on its spec branch and draft spec PR, starting with the first unblocked one on the team's Dev."
            : "Loft opens a ticket/\(n) branch and a draft PR that closes #\(n), then has the team's Dev build it and QA, Security and Code Review check it."
    }

    private func color(_ tone: IssueDelivery.Tone) -> Color {
        switch tone {
        case .running, .final: Theme.accent
        case .stalled: Theme.failureInk
        case .paused: .secondary
        case .needsYou: Theme.attentionInk
        case .done: Theme.working
        }
    }

    /// A card button: a spinner in place of its icon while its request runs, and every button disabled meanwhile.
    private func button(_ action: IssueDeliveryAction, _ title: String, _ pendingTitle: String, symbol: String) -> some View {
        Button { run(action) } label: {
            Label {
                Text(busy == action ? pendingTitle : title)
            } icon: {
                if busy == action { ProgressView().controlSize(.mini) } else { Image(systemName: symbol) }
            }
        }
        .disabled(busy != nil || !store.canWrite)
        .accessibilityIdentifier("delivery-\(action.rawValue)")
    }

    private func run(_ action: IssueDeliveryAction) {
        busy = action
        said = nil
        Task {
            do {
                let message = try await store.deliveryAction(task.id, action)
                said = message.map { (true, $0) }
            } catch {
                said = (false, APIError.from(error).localizedDescription)
            }
            busy = nil
        }
    }
}
