import SwiftUI

/// The top of a spec's detail: its Delivery (state, who has which ticket, progress, spec PR), its tickets with their
/// verdicts, and Merge for a plan kept in Loft. A spec without a Delivery gets no panel.
/// Under the Delivery card (an open spec on GitHub) the card has the state, who has which ticket and Start delivery, so
/// this adds only the Spec PR, progress and tickets.
struct SpecDeliverySection: View {
    @Environment(LoftStore.self) private var store
    let spec: HBTask
    var underCard = false
    @State private var merging = false
    @State private var error: String?
    @State private var merged: String?

    private var delivery: DeliverySummary? { SpecViews.delivery(of: spec, in: store.deliverySummaries) }

    var body: some View {
        let d = delivery
        let stage = SpecViews.stage(of: spec, d)
        let tickets = SpecViews.tickets(of: spec, d, tasks: store.tasks)
        let started = d.map { $0.state != "planned" } ?? false
        Group {
            if let d {
                Section {
                    // Under the card with no spec PR yet, the row would be empty.
                    if !underCard || d.pr != nil {
                        HStack {
                            if !underCard {
                                Text(SpecViews.deliveryLabel(d.state)).font(.subheadline.weight(.semibold)).foregroundStyle(stage == .ready ? Theme.attention : Theme.accent)
                                    .accessibilityIdentifier("deliveryState")
                            }
                            Spacer()
                            if let pr = d.pr, let g = spec.github, let url = URL(string: "https://github.com/\(g.repo)/pull/\(pr)") {
                                Link(destination: url) {
                                    HStack(spacing: 4) { Image(systemName: "arrow.triangle.pull"); Text("Spec PR #\(pr)") }.font(.subheadline)
                                }
                                .buttonStyle(.borderless)
                                    .accessibilityIdentifier("specPRLink")
                            }
                        }
                    }
                    if !underCard {
                        if started {
                            ForEach(SpecViews.onIt(d), id: \.self) { Text($0).font(.subheadline).foregroundStyle(.secondary).accessibilityIdentifier("deliveryOnIt") }
                            if let line = SpecViews.waitingOnYouLine(d.waitingOnYou) {
                                Text(line).font(.subheadline).foregroundStyle(Theme.attentionInk).accessibilityIdentifier("deliveryWaitingOnYou")
                            }
                            if let waiting = SpecViews.waitingLine(d.waitingForLead) {
                                Text(waiting).font(.subheadline).foregroundStyle(Theme.attentionInk).accessibilityIdentifier("deliveryWaiting")
                            }
                        } else {
                            Text("Not started: no ticket has been handed to an agent yet.").font(.subheadline).foregroundStyle(.secondary)
                        }
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
                } header: { Text(underCard ? "Progress" : "Delivery") }
            }
            if SpecViews.canMerge(spec, d) || merged != nil || error != nil {
                Section {
                    if let d, SpecViews.canMerge(spec, d) {
                        Button { merge(d) } label: {
                            if merging { HStack { ProgressView(); Text("Merging…") } } else { Label("Merge", systemImage: "arrow.triangle.merge") }
                        }
                        .disabled(merging || !store.canWrite)
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

    private func merge(_ d: DeliverySummary) {
        merging = true; error = nil
        Task {
            do { merged = try await store.mergeDelivery(d) } catch { self.error = APIError.from(error).localizedDescription }
            merging = false
        }
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
                Text(SpecViews.ticketLabel(ticket.state, waitingOn: ticket.waitingOn)).font(.caption.weight(.medium)).foregroundStyle(tone)
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
        if ticket.state == "open" && !ticket.waitingOn.isEmpty { return Theme.attention }
        return switch ticket.state {
        case "closed": Theme.working
        case "needs_human": Theme.attention
        case "started", "review", "closing": Theme.accent
        default: .secondary
        }
    }
}
