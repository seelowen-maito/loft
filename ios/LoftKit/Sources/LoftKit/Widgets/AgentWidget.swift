import AppIntents
import SwiftUI
import WidgetKit

/// An agent the One agent widget can show. A deleted one comes back without a name; the widget remembers the name it last drew.
public struct AgentEntity: AppEntity {
    public static let typeDisplayRepresentation: TypeDisplayRepresentation = "Agent"
    public static var defaultQuery: AgentEntityQuery { AgentEntityQuery() }

    public let id: String
    public let name: String

    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }

    public var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(name)") }
}

/// The user's agents, for the widget's agent picker.
public struct AgentEntityQuery: EntityQuery {
    public init() {}

    public func entities(for identifiers: [String]) async throws -> [AgentEntity] {
        let agents = (try? await Self.agents()) ?? []
        // One Loft no longer has (deleted, or Loft is offline) stays picked: the widget shows it gone or offline.
        return identifiers.map { id in agents.first { $0.id == id }.map { AgentEntity(id: $0.id, name: $0.name) } ?? AgentEntity(id: id, name: "") }
    }

    public func suggestedEntities() async throws -> [AgentEntity] { Self.entities(try await Self.agents()) }

    /// Every agent, by name.
    static func entities(_ agents: [Agent]) -> [AgentEntity] {
        agents.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }.map { AgentEntity(id: $0.id, name: $0.name) }
    }

    private static func agents() async throws -> [Agent] {
        guard let server = SharedServer.address else { return [] }
        return try await LiveLoftAPI(baseURL: server).agents()
    }
}

/// The One agent widget's settings: which agent.
public struct AgentWidgetIntent: WidgetConfigurationIntent {
    public static let title: LocalizedStringResource = "Agent"
    public static let description = IntentDescription("Pick the agent the widget shows.")

    @Parameter(title: "Agent") public var agent: AgentEntity?

    public init() {}
}

/// LoftKit's App Intents, which the widget extensions include so the system finds them.
public struct LoftKitIntents: AppIntentsPackage {}

struct AgentEntry: TimelineEntry {
    let date: Date
    let snapshot: AgentWidgetSnapshot
}

struct AgentProvider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> AgentEntry { AgentEntry(date: .now, snapshot: .sample) }

    func snapshot(for configuration: AgentWidgetIntent, in context: Context) async -> AgentEntry {
        if context.isPreview { return placeholder(in: context) }
        return await entry(configuration)
    }

    /// Reads Loft again every 15 minutes (the system decides when it really runs).
    func timeline(for configuration: AgentWidgetIntent, in context: Context) async -> Timeline<AgentEntry> {
        Timeline(entries: [await entry(configuration)], policy: .after(.now.addingTimeInterval(15 * 60)))
    }

    private func entry(_ configuration: AgentWidgetIntent) async -> AgentEntry {
        AgentEntry(date: .now, snapshot: await AgentWidgetSnapshot.load(agentId: configuration.agent?.id, name: configuration.agent?.name ?? ""))
    }
}

/// One agent the user picks: its status, ticket, last message and run time (spec #462). Small and medium, on the
/// iPhone and the Mac; a tap opens its chat.
public struct AgentWidget: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "loft.agent", intent: AgentWidgetIntent.self, provider: AgentProvider()) { entry in
            AgentEntryView(entry: entry)
                .containerBackground(.background, for: .widget)
                .widgetURL(entry.snapshot.link.url)
        }
        .configurationDisplayName("Agent")
        .description("What one agent is doing: its status, ticket, last message and run time.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

private struct AgentEntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: AgentEntry

    var body: some View { AgentWidgetView(entry.snapshot, size: family == .systemSmall ? .small : .medium) }
}

extension AgentWidgetSnapshot {
    /// Where a tap goes: the agent's chat, or the agents list when there's no agent to open.
    var link: DeepLink { phase == .ready || phase == .unreachable ? .agent(agentId) : .agents }

    /// What the gallery and the placeholder show.
    static var sample: AgentWidgetSnapshot {
        var s = AgentWidgetSnapshot(phase: .ready, agentId: "dev", name: "Dev")
        s.state = .working
        s.ticket = "#465 One-agent widget"
        s.lastMessage = "Tests are green. Pushing the widget to the spec branch now."
        s.runStart = .now.addingTimeInterval(-143)
        return s
    }
}

/// The One agent widget's contents, small and medium.
public struct AgentWidgetView: View {
    public enum Size { case small, medium }

    let snapshot: AgentWidgetSnapshot
    let size: Size

    public init(_ snapshot: AgentWidgetSnapshot, size: Size) {
        self.snapshot = snapshot
        self.size = size
    }

    public var body: some View {
        switch snapshot.phase {
        case .notConnected: LoftWidgetView(.notConnected, size: .small)
        case .unreachable:
            if snapshot.name.isEmpty {
                LoftWidgetView(.unreachable, size: .small)
            } else {
                // Which agent, so two Agent widgets (and the Needs you one) still look different while Loft is down.
                VStack(alignment: .leading, spacing: 4) {
                    face(36)
                    Spacer(minLength: 0)
                    name
                    Label("Can't reach Loft", systemImage: "bolt.horizontal.circle").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            }
        case .notChosen: message("Pick an agent", "Edit the widget to choose who it shows.", symbol: "person.crop.circle.badge.plus")
        case .gone:
            message("Agent gone", "\(snapshot.name.isEmpty ? "This agent" : snapshot.name) was deleted. Edit the widget to pick another.",
                    symbol: "person.crop.circle.badge.xmark")
        case .ready: ready
        }
    }

    private func message(_ title: String, _ detail: String, symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Image(systemName: symbol).font(.title2).foregroundStyle(WidgetTheme.accent)
            Text(title).font(.headline)
            Text(detail).font(.caption).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }

    @ViewBuilder private var ready: some View {
        if size == .small {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .top, spacing: 6) {
                    face(32)
                    Text(snapshot.lastMessage ?? "No messages yet.").font(.caption2).foregroundStyle(.secondary).lineLimit(3)
                }
                Spacer(minLength: 0)
                name
                status
                if let ticket = snapshot.ticket { Text(ticket).font(.caption2).foregroundStyle(.secondary).lineLimit(1) }
                runTime.font(.caption2.monospacedDigit()).foregroundStyle(.secondary).minimumScaleFactor(0.75)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        } else {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    face(44)
                    Spacer(minLength: 0)
                    name
                    status
                }
                .frame(width: 124, alignment: .leading)
                VStack(alignment: .leading, spacing: 4) {
                    if let ticket = snapshot.ticket {
                        Label(ticket, systemImage: "smallcircle.filled.circle").font(.caption.weight(.semibold)).lineLimit(1)
                    }
                    Text(snapshot.lastMessage ?? "No messages yet.").font(.caption).foregroundStyle(.secondary).lineLimit(3)
                    Spacer(minLength: 0)
                    runTime.font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
        }
    }

    /// The whole name, on two lines and smaller if it must: it's what tells two agent widgets apart.
    private var name: some View {
        Text(snapshot.name).font(.headline).lineLimit(2).minimumScaleFactor(0.7).layoutPriority(1)
    }

    private var status: some View {
        Label(snapshot.state.label, systemImage: snapshot.state.symbol).font(.caption).foregroundStyle(snapshot.state.color).lineLimit(1)
    }

    /// A live timer while it works, else how long its last run took.
    @ViewBuilder private var runTime: some View {
        if let start = snapshot.runStart {
            Text("Running \(Text(start, style: .timer))").lineLimit(1)
        } else if let last = snapshot.lastRun {
            Text(last).lineLimit(1)
        }
    }

    /// The agent's drawing, else its initials on a disc.
    @ViewBuilder private func face(_ points: CGFloat) -> some View {
        if let svg = snapshot.avatarSVG, let image = SVGDocument(svg)?.image(pixels: Int(points * 3)) {
            Image(decorative: image, scale: 3).resizable().frame(width: points, height: points)
        } else {
            let initials = String(snapshot.name.split(separator: " ").prefix(2).compactMap(\.first)).uppercased()
            Text(initials).font(.system(size: points * 0.38, weight: .bold, design: .rounded))
                .frame(width: points, height: points)
                .background(WidgetTheme.accent.opacity(0.2), in: Circle())
        }
    }
}
