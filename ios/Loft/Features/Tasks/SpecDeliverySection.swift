import SwiftUI

/// The top of a spec's detail: its Delivery (state, who has which ticket, progress, spec PR), its tickets with their
/// verdicts, and Start delivery or Merge where the web shows them. A spec without a Delivery gets no panel, only Start delivery.
struct SpecDeliverySection: View {
    @Environment(LoftStore.self) private var store
    let spec: HBTask
    @State private var busy: Busy?
    @State private var error: String?
    @State private var merged: String?

    private enum Busy { case start, merge }

    private var delivery: DeliverySummary? { SpecViews.delivery(of: spec, in: store.deliveries) }

    var body: some View {
        let d = delivery
        let stage = SpecViews.stage(of: spec, d)
        let tickets = SpecViews.tickets(of: spec, d, tasks: store.tasks)
        let started = d.map { $0.state != "planned" } ?? false
        Group {
            if let d {
                Section {
                    HStack {
                        Text(Self.stateLabel(d.state)).font(.subheadline.weight(.semibold)).foregroundStyle(stage == .ready ? Theme.attention : Theme.accent)
                            .accessibilityIdentifier("deliveryState")
                        Spacer()
                        if let pr = d.pr, let g = spec.github, let url = URL(string: "https://github.com/\(g.repo)/pull/\(pr)") {
                            Link(destination: url) {
                                HStack(spacing: 4) { Image(systemName: "arrow.triangle.pull"); Text("Spec PR #\(pr)") }.font(.subheadline)
                            }
                            .buttonStyle(.borderless)
                                .accessibilityIdentifier("specPRLink")
                        }
                    }
                    if started {
                        if let who = Self.onIt(d) { Text(who).font(.subheadline).foregroundStyle(.secondary).accessibilityIdentifier("deliveryOnIt") }
                    } else {
                        Text("Not started: no ticket has been handed to an agent yet.").font(.subheadline).foregroundStyle(.secondary)
                    }
                    if let p = SpecViews.progress(of: spec, d, tasks: store.tasks) {
                        HStack(spacing: 8) {
                            ProgressView(value: Double(p.done), total: Double(max(p.total, 1))).tint(Theme.accent)
                            Text("\(p.done)/\(p.total)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        }
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("\(p.done) of \(p.total) tickets done")
                        .accessibilityIdentifier("deliveryProgress")
                    }
                } header: { Text("Delivery") }
            }
            if SpecViews.canStartDelivery(spec, d) || SpecViews.canMerge(spec, d) || merged != nil || error != nil {
                Section {
                    if SpecViews.canStartDelivery(spec, d) {
                        Button { start() } label: {
                            if busy == .start { HStack { ProgressView(); Text("Starting delivery…") } } else { Label("Start delivery", systemImage: "play.fill") }
                        }
                        .disabled(busy != nil || !store.canWrite)
                        .accessibilityIdentifier("startDelivery")
                    }
                    if let d, SpecViews.canMerge(spec, d) {
                        Button { merge(d) } label: {
                            if busy == .merge { HStack { ProgressView(); Text("Merging…") } } else { Label("Merge", systemImage: "arrow.triangle.merge") }
                        }
                        .disabled(busy != nil || !store.canWrite)
                        .accessibilityIdentifier("mergeDelivery")
                    }
                    if let merged { Label(merged, systemImage: "checkmark.circle").font(.footnote).foregroundStyle(Theme.working).accessibilityIdentifier("deliveryResult") }
                    if let error { Text(error).font(.footnote).foregroundStyle(Theme.failure) }
                }
            }
            if !tickets.isEmpty {
                Section {
                    ForEach(tickets) { t in
                        if let id = t.taskId {
                            NavigationLink(value: TaskRoute(id: id)) { TicketRow(ticket: t) }
                        } else {
                            TicketRow(ticket: t)
                        }
                    }
                } header: { Text("Tickets") }
            }
        }
        .onAppear { store.deliveryWatchers += 1; store.loadDeliveries() }
        .onDisappear { store.deliveryWatchers -= 1 }
    }

    private func start() {
        busy = .start; error = nil
        Task {
            do { try await store.startDelivery(spec: spec.id) } catch { self.error = APIError.from(error).localizedDescription }
            busy = nil
        }
    }

    private func merge(_ d: DeliverySummary) {
        busy = .merge; error = nil
        Task {
            do { merged = try await store.mergeDelivery(d) } catch { self.error = APIError.from(error).localizedDescription }
            busy = nil
        }
    }

    static func stateLabel(_ state: String) -> String {
        switch state {
        case "planned": "Not started"
        case "running": "Running"
        case "needs_human": "Needs you"
        case "ready", "settled": "Ready to merge"
        case "merged": "Merged"
        default: state.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    /// "Backend Agent on #43 Backend endpoints".
    static func onIt(_ d: DeliverySummary) -> String? {
        let parts = [d.agentName, d.current.map { "#\($0.number) \($0.title)" }].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " on ")
    }
}

private struct TicketRow: View {
    let ticket: SpecViews.TicketLine

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if let n = ticket.number { Text("#\(n)").font(.caption.monospacedDigit()).foregroundStyle(.secondary) }
                Text(ticket.title).readableLineLimit(2)
                Spacer(minLength: 4)
                Text(SpecViews.ticketLabel(ticket.state)).font(.caption.weight(.medium)).foregroundStyle(tone)
            }
            if !ticket.verdicts.isEmpty {
                HStack(spacing: 8) {
                    ForEach(ticket.verdicts.sorted { $0.key < $1.key }, id: \.key) { role, v in
                        HStack(spacing: 3) {
                            Image(systemName: v == "pass" ? "checkmark.circle.fill" : "clock")
                            Text("\(role) \(v == "pass" ? "passed" : "waiting")")
                        }
                        .font(.caption2).foregroundStyle(v == "pass" ? Theme.working : .secondary)
                    }
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("ticketRow-\(ticket.title)")
    }

    private var tone: Color {
        switch ticket.state {
        case "closed": Theme.working
        case "needs_human": Theme.attention
        case "started", "review", "closing": Theme.accent
        default: .secondary
        }
    }
}
