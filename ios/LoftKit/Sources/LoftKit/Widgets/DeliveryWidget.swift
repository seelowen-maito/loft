import AppIntents
import SwiftUI
import WidgetKit

/// A spec's Delivery the Delivery progress widget can show, by its key (`DeliveryWidgetSnapshot.key`). A gone one
/// comes back without a title; the widget remembers the title it last drew.
public struct DeliveryEntity: AppEntity {
    public static let typeDisplayRepresentation: TypeDisplayRepresentation = "Delivery"
    public static var defaultQuery: DeliveryEntityQuery { DeliveryEntityQuery() }

    public let id: String
    public let title: String
    /// Its repo, so two repos' specs with one number tell apart in the picker.
    public let repo: String

    public init(id: String, title: String, repo: String = "") {
        self.id = id
        self.title = title
        self.repo = repo
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", subtitle: repo.isEmpty ? nil : "\(repo)")
    }
}

/// The running and recent Deliveries, for the widget's picker.
public struct DeliveryEntityQuery: EntityQuery {
    public init() {}

    public func entities(for identifiers: [String]) async throws -> [DeliveryEntity] {
        let offered = (try? await Self.choices()) ?? []
        // One Loft no longer has (or Loft is offline) stays picked: the widget shows it gone or offline.
        return identifiers.map { id in offered.first { $0.id == id } ?? DeliveryEntity(id: id, title: "") }
    }

    public func suggestedEntities() async throws -> [DeliveryEntity] { try await Self.choices() }

    private static func choices() async throws -> [DeliveryEntity] {
        guard let server = SharedServer.address else { return [] }
        let api = LiveLoftAPI(baseURL: server)
        async let deliveries = api.deliverySummaries()
        async let tasks = try? api.tasks()
        return DeliveryWidgetSnapshot.choices(try await deliveries, tasks: await tasks ?? []).map { DeliveryEntity(id: $0.key, title: $0.title, repo: $0.repo) }
    }
}

/// The Delivery progress widget's settings: which spec.
public struct DeliveryWidgetIntent: WidgetConfigurationIntent {
    public static let title: LocalizedStringResource = "Delivery"
    public static let description = IntentDescription("Pick the spec whose Delivery the widget follows.")

    @Parameter(title: "Delivery") public var delivery: DeliveryEntity?

    public init() {}
}

struct DeliveryEntry: TimelineEntry {
    let date: Date
    let snapshot: DeliveryWidgetSnapshot
}

struct DeliveryProvider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> DeliveryEntry { DeliveryEntry(date: .now, snapshot: .sample) }

    func snapshot(for configuration: DeliveryWidgetIntent, in context: Context) async -> DeliveryEntry {
        if context.isPreview { return placeholder(in: context) }
        return await entry(configuration)
    }

    /// Reads Loft again every 15 minutes (the system decides when it really runs).
    func timeline(for configuration: DeliveryWidgetIntent, in context: Context) async -> Timeline<DeliveryEntry> {
        Timeline(entries: [await entry(configuration)], policy: .after(.now.addingTimeInterval(15 * 60)))
    }

    private func entry(_ configuration: DeliveryWidgetIntent) async -> DeliveryEntry {
        DeliveryEntry(date: .now, snapshot: await DeliveryWidgetSnapshot.load(key: configuration.delivery?.id, title: configuration.delivery?.title ?? ""))
    }
}

/// One spec's Delivery the user picks: tickets closed out of the total, the ticket being built and its Dev, and the
/// reviews' verdicts (spec #462). Medium, on the iPhone and the Mac; a tap opens the spec.
public struct DeliveryWidget: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "loft.delivery", intent: DeliveryWidgetIntent.self, provider: DeliveryProvider()) { entry in
            DeliveryWidgetView(entry.snapshot)
                .containerBackground(.background, for: .widget)
                .widgetURL(entry.snapshot.link.url)
        }
        .configurationDisplayName("Delivery")
        .description("How a spec's Delivery is going: tickets done, who builds which, and the reviews.")
        .supportedFamilies([.systemMedium])
    }
}

extension DeliveryWidgetSnapshot {
    /// Where a tap goes: the spec's page; without it (Loft unreachable, the Delivery gone, its issue not listed)
    /// Needs you, where a Delivery waiting on you shows, rather than an unrelated list.
    var link: DeepLink { specTaskId.map { .issue($0) } ?? .needsYou }

    /// What the gallery and the placeholder show.
    static var sample: DeliveryWidgetSnapshot {
        var s = DeliveryWidgetSnapshot(phase: .ready, key: "loft#462", title: "#462 Widgets: Needs you, One agent, Delivery, Talk")
        s.done = 2
        s.total = 4
        s.ticket = "#466 Delivery progress widget"
        s.dev = "Loft Dev"
        s.qa = .passed
        return s
    }
}

/// The Delivery progress widget's contents.
public struct DeliveryWidgetView: View {
    let snapshot: DeliveryWidgetSnapshot

    public init(_ snapshot: DeliveryWidgetSnapshot) { self.snapshot = snapshot }

    public var body: some View {
        switch snapshot.phase {
        case .notConnected: LoftWidgetView(.notConnected, size: .small)
        case .unreachable:
            if snapshot.title.isEmpty {
                LoftWidgetView(.unreachable, size: .small)
            } else {
                message(snapshot.title, "Can't reach Loft. Check that it's running and reachable.", symbol: "bolt.horizontal.circle")
            }
        case .notChosen: message("Pick a Delivery", "Edit the widget to choose which spec it follows.", symbol: "shippingbox")
        case .gone:
            message("Delivery gone", "\(snapshot.title.isEmpty ? "This spec" : snapshot.title) has no Delivery any more. Edit the widget to pick another.",
                    symbol: "shippingbox.and.arrow.backward")
        case .ready: ready
        }
    }

    private func message(_ title: String, _ detail: String, symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Image(systemName: symbol).font(.title2).foregroundStyle(WidgetTheme.accent)
            Text(title).font(.headline).lineLimit(2)
            Text(detail).font(.caption).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }

    private var ready: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(snapshot.title).font(.headline).lineLimit(2).minimumScaleFactor(0.8)
                Spacer(minLength: 4)
                Label(snapshot.status.label, systemImage: symbol).font(.caption.weight(.semibold)).foregroundStyle(color).lineLimit(1)
                    .fixedSize()
            }
            Spacer(minLength: 0)
            // A bar of its own: ImageRenderer, which draws the tests' screenshots, can't draw a ProgressView.
            GeometryReader { g in
                Capsule().fill(.secondary.opacity(0.25))
                    .overlay(alignment: .leading) {
                        Capsule().fill(color).frame(width: g.size.width * Double(snapshot.done) / Double(max(snapshot.total, 1)))
                    }
            }
            .frame(height: 6)
            Text(snapshot.progress).font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
            detail.font(.caption).lineLimit(1)
            HStack(spacing: 12) {
                verdict("Review", snapshot.review)
                verdict("QA", snapshot.qa)
                verdict("Security", snapshot.security)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    /// The ticket being built and its Dev, or what the state means when nothing is.
    @ViewBuilder private var detail: some View {
        switch snapshot.status {
        case .stalled: Text("Stalled: open the spec to resume it.").foregroundStyle(color)
        case .paused: Text("Paused: Loft sends no wakes until you resume it.").foregroundStyle(.secondary)
        case .done: Text("Every ticket closed and reviewed: ready to merge.").foregroundStyle(.secondary)
        case .merged: Text("The spec PR is merged.").foregroundStyle(.secondary)
        default:
            if snapshot.status == .needsYou, let waiting = snapshot.waitingOnYou {
                Text("\(Text(waiting).fontWeight(.semibold)) waits for you").foregroundStyle(color)
            } else if let ticket = snapshot.ticket {
                Text("\(Text(ticket).fontWeight(.semibold))\(snapshot.dev.map { " · \($0)" } ?? "")")
            } else {
                Text(snapshot.status == .needsYou ? "A ticket waits for you." : "Waiting for the next ticket.").foregroundStyle(.secondary)
            }
        }
    }

    private func verdict(_ name: String, _ v: DeliveryWidgetSnapshot.Verdict) -> some View {
        let (symbol, tint): (String, Color) = switch v {
        case .passed: ("checkmark.circle.fill", WidgetTheme.working)
        case .failed: ("xmark.circle.fill", Theme.failure)
        case .pending: ("circle.dotted", .secondary)
        }
        return Label { Text(name).foregroundStyle(.secondary) } icon: { Image(systemName: symbol).foregroundStyle(tint) }
            .font(.caption2).lineLimit(1)
    }

    private var symbol: String {
        switch snapshot.status {
        case .running: "play.circle.fill"
        case .finalReview: "eye.circle.fill"
        case .needsYou: "hand.raised.circle.fill"
        case .stalled: "exclamationmark.triangle.fill"
        case .paused: "pause.circle.fill"
        case .done, .merged: "checkmark.seal.fill"
        }
    }

    private var color: Color {
        switch snapshot.status {
        case .running, .finalReview: WidgetTheme.accent
        case .needsYou, .paused: WidgetTheme.attention
        case .stalled: Theme.failure
        case .done, .merged: WidgetTheme.working
        }
    }
}
