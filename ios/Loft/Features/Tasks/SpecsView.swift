import SwiftUI

/// The Issues tab's Specs view: the specs it's given (already filtered), in sections by stage.
struct SpecsView<Header: View>: View {
    @Environment(LoftStore.self) private var store
    let specs: [HBTask]
    let showDropped: Bool
    /// Clears the filters that hide specs; nil when none is set.
    var clearFilters: (() -> Void)?
    @ViewBuilder let header: Header

    private func rows(_ stage: SpecStage) -> [HBTask] {
        specs.filter { SpecViews.stage(of: $0, SpecViews.delivery(of: $0, in: store.deliverySummaries)) == stage }
            .sorted { $0.updatedAt > $1.updatedAt }
    }

    private var stages: [SpecStage] { SpecStage.allCases.filter { $0 != .dropped || showDropped } }

    var body: some View {
        List {
            StaleBanner().listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
            header
            ForEach(stages) { stage in
                let items = rows(stage)
                if !items.isEmpty {
                    Section {
                        ForEach(items) { spec in
                            NavigationLink(value: TaskRoute(id: spec.id)) { SpecRow(spec: spec) }
                        }
                    } header: {
                        Text("\(stage.label) · \(items.count)").accessibilityIdentifier("specSection-\(stage.rawValue)")
                    }
                }
            }
        }
        .overlay {
            if !store.hasLoaded {
                NotLoadedView()
            } else if stages.allSatisfy({ rows($0).isEmpty }), let clearFilters {
                EmptyState(symbol: "line.3.horizontal.decrease", title: "No matching specs", message: "No specs match the filters.",
                           actionTitle: "Clear filters", action: clearFilters)
            } else if stages.allSatisfy({ rows($0).isEmpty }) {
                EmptyState(symbol: "doc.text", title: "No specs", message: "Specs you plan show up here, by stage.")
            }
        }
        .refreshable { try? await store.refresh(); store.loadDeliveries() }
        .onAppear { store.deliveryWatchers += 1; store.loadDeliveries() }
        .onDisappear { store.deliveryWatchers -= 1 }
    }
}

struct SpecRow: View {
    @Environment(LoftStore.self) private var store
    let spec: HBTask

    private var delivery: DeliverySummary? { SpecViews.delivery(of: spec, in: store.deliverySummaries) }
    private var stage: SpecStage { SpecViews.stage(of: spec, delivery) }

    private var badge: String? { delivery.map { SpecViews.deliveryLabel($0.state) } }

    /// Each running ticket with its agent, then the tickets waiting for the Lead.
    private var onIt: [String] {
        guard stage == .delivery, let d = delivery else { return [] }
        return SpecViews.onIt(d) + [SpecViews.waitingOnYouLine(d.waitingOnYou), SpecViews.waitingLine(d.waitingForLead)].compactMap { $0 }
    }

    var body: some View {
        let p = SpecViews.progress(of: spec, delivery, tasks: store.tasks)
        VStack(alignment: .leading, spacing: 5) {
            Text(spec.title).font(.body).readableLineLimit(2)
            HStack(spacing: 6) {
                Text(SpecViews.ref(spec)).font(.caption).foregroundStyle(.secondary).readableLineLimit(1)
                if let badge {
                    Text(badge).font(.caption2.weight(.medium))
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(Theme.accent.opacity(0.15), in: Capsule())
                        .foregroundStyle(Theme.accent)
                }
            }
            if let p {
                HStack(spacing: 8) {
                    ProgressView(value: Double(p.done), total: Double(max(p.total, 1))).tint(Theme.accent)
                    Text("\(p.done)/\(p.total)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("\(p.done) of \(p.total) tickets done")
            }
            ForEach(onIt, id: \.self) { line in
                Text(line).font(.caption).foregroundStyle(.secondary).readableLineLimit(2)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("specRow-\(spec.title)")
    }
}
