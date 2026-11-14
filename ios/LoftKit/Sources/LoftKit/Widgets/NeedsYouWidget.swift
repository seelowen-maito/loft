import AppIntents
import SwiftUI
import WidgetKit

/// Deny on the Needs you widget (spec #462): sends the permission item's own action, as the app's Needs you does,
/// without opening the app. WidgetKit reloads the widget once it returns, so the answered item drops off. Approve is a
/// link to the item's card in the app instead.
public struct NeedsYouDecisionIntent: AppIntent {
    public static let title: LocalizedStringResource = "Answer a permission request"
    public static let isDiscoverable = false

    @Parameter(title: "Method") public var method: String
    @Parameter(title: "Path") public var path: String
    /// The action's body as JSON text: an intent's parameters can't hold a `JSONValue`.
    @Parameter(title: "Body") public var body: String

    public init() {}

    public init(_ action: NeedsYouAction) {
        method = action.method ?? "POST"
        path = action.path ?? ""
        body = action.body.flatMap { try? String(data: JSONEncoder().encode($0), encoding: .utf8) } ?? ""
    }

    public func perform() async throws -> some IntentResult {
        guard let server = SharedServer.address else { return .result() }
        try await send(LiveLoftAPI(baseURL: server).needsYouRequest) { WidgetCenter.shared.reloadAllTimelines() }
        return .result()
    }

    /// Sends the action, then reloads the widgets (the overview counts the same items) even when it failed: a request
    /// answered elsewhere drops off instead of keeping buttons that do nothing, and an offline Loft shows as offline.
    func send(_ request: (String, String, JSONValue?) async throws -> Void, reload: () -> Void) async throws {
        defer { reload() }
        try await send(request)
    }

    /// Sends the action through `request` (the API's `needsYouRequest`).
    func send(_ request: (String, String, JSONValue?) async throws -> Void) async throws {
        let json = body.isEmpty ? nil : try JSONDecoder().decode(JSONValue.self, from: Data(body.utf8))
        try await request(method, path, json)
    }
}

/// What needs you, most urgent first, with Approve and Deny on permission items (spec #462). Medium and large on the
/// iPhone and the Mac, and a count on the iPhone's Lock Screen.
public struct NeedsYouWidget: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: "loft.needs-you", provider: LoftProvider()) { entry in
            NeedsYouEntryView(entry: entry)
                .containerBackground(.background, for: .widget)
                .widgetURL(DeepLink.needsYou.url)
        }
        .configurationDisplayName("Needs you")
        .description("Deny permission requests or open them to approve, and see what else waits on you.")
        .supportedFamilies(Self.families)
    }

    private static var families: [WidgetFamily] {
        #if os(iOS)
        [.systemMedium, .systemLarge, .accessoryCircular, .accessoryInline]
        #else
        [.systemMedium, .systemLarge]
        #endif
    }
}

private struct NeedsYouEntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: LoftEntry

    var body: some View {
        switch family {
        #if os(iOS)
        case .accessoryCircular, .accessoryInline: NeedsYouCountView(snapshot: entry.snapshot, inline: family == .accessoryInline)
        #endif
        default: NeedsYouWidgetView(entry.snapshot, size: family == .systemLarge ? .large : .medium)
        }
    }
}

/// The Needs you widget's list, medium and large.
public struct NeedsYouWidgetView: View {
    public enum Size { case medium, large }

    let snapshot: WidgetSnapshot
    let size: Size

    public init(_ snapshot: WidgetSnapshot, size: Size) {
        self.snapshot = snapshot
        self.size = size
    }

    /// Rows that fit: a medium widget has room for two.
    private var items: ArraySlice<WidgetSnapshot.Item> { snapshot.items.prefix(size == .large ? WidgetSnapshot.itemLimit : 2) }

    public var body: some View {
        if snapshot.phase != .ready {
            // Not connected or offline: the overview's message.
            LoftWidgetView(snapshot, size: .medium)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text("\(snapshot.needsCount)").font(.system(size: 24, weight: .bold, design: .rounded))
                        .foregroundStyle(snapshot.urgent > 0 ? WidgetTheme.attention : WidgetTheme.accent)
                    Text("Needs you").font(.subheadline.weight(.semibold))
                    Spacer(minLength: 0)
                    if snapshot.needsCount > items.count {
                        Text("\(snapshot.needsCount - items.count) more").font(.caption2).foregroundStyle(.secondary)
                    }
                }
                .widgetLink(.needsYou)
                if items.isEmpty {
                    Text("Nothing is waiting on you.").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(items) { row($0) }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private func row(_ item: WidgetSnapshot.Item) -> some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 1) {
                Text(item.title).font(.caption.weight(.semibold)).lineLimit(1)
                if item.deny != nil {
                    // What it runs (the title names the agent); the app shows it whole before Approve.
                    Text(item.summary).font(.caption2.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                } else {
                    Text([item.agentName, item.summary].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .widgetLink(item.link)
            // Only the buttons are bordered: a Link takes the button style too.
            Group {
                if let deny = item.deny {
                    Button(intent: NeedsYouDecisionIntent(deny)) { Image(systemName: "xmark") }
                        .tint(.secondary).accessibilityLabel("Deny")
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
            if let approve = item.approve {
                // Opens the request's card in the app: Approve there, beside the whole command.
                Image(systemName: "checkmark").font(.caption.weight(.semibold)).foregroundStyle(WidgetTheme.accent)
                    .frame(width: 34, height: 24).background(WidgetTheme.accent.opacity(0.22), in: Capsule())
                    .widgetLink(approve)
                    .accessibilityLabel("Approve in Loft")
            }
        }
    }
}

/// The Lock Screen's count.
private struct NeedsYouCountView: View {
    let snapshot: WidgetSnapshot
    let inline: Bool

    var body: some View {
        let count = snapshot.phase == .ready ? "\(snapshot.needsCount)" : "–"
        let words = switch snapshot.phase {
        case .ready: snapshot.needsCount == 1 ? "1 needs you" : "\(count) need you"
        case .notConnected: "Not connected"
        case .unreachable: "Can't reach Loft"
        }
        if inline {
            Label(words, systemImage: "bell")
        } else {
            ZStack {
                AccessoryWidgetBackground()
                VStack(spacing: -2) {
                    Text(count).font(.system(.title2, design: .rounded).weight(.bold))
                    Image(systemName: "bell.fill").font(.caption2)
                }
            }
            .accessibilityLabel(words)
        }
    }
}
