import SwiftUI
import WidgetKit

struct LoftEntry: TimelineEntry {
    let date: Date
    let snapshot: WidgetSnapshot
}

struct LoftProvider: TimelineProvider {
    func placeholder(in context: Context) -> LoftEntry { LoftEntry(date: .now, snapshot: .sample) }

    func getSnapshot(in context: Context, completion: @escaping (LoftEntry) -> Void) {
        if context.isPreview { return completion(placeholder(in: context)) }
        Task { completion(LoftEntry(date: .now, snapshot: await WidgetSnapshot.load())) }
    }

    /// Reads Loft again every 15 minutes (the system decides when it really runs).
    func getTimeline(in context: Context, completion: @escaping (Timeline<LoftEntry>) -> Void) {
        Task {
            let entry = LoftEntry(date: .now, snapshot: await WidgetSnapshot.load())
            completion(Timeline(entries: [entry], policy: .after(.now.addingTimeInterval(15 * 60))))
        }
    }
}

/// Needs you, who is working and the spend, small and medium (spec #382). The widget extensions of both apps
/// put it in their `@main` bundle.
public struct LoftOverviewWidget: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: "loft.overview", provider: LoftProvider()) { entry in
            EntryView(entry: entry)
                .containerBackground(.background, for: .widget)
                // A tap anywhere without a link of its own (all of the small one) opens Needs you.
                .widgetURL(DeepLink.needsYou.url)
        }
        .configurationDisplayName("Loft")
        .description("What needs you, who is working and the week's spend.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

private struct EntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: LoftEntry

    var body: some View { LoftWidgetView(entry.snapshot, size: family == .systemSmall ? .small : .medium) }
}

extension WidgetSnapshot {
    /// What the gallery and the placeholder show.
    static var sample: WidgetSnapshot {
        var s = WidgetSnapshot(phase: .ready)
        s.needsCount = 3
        s.urgent = 1
        let deny = NeedsYouAction(kind: "deny", label: "Deny", method: "POST", path: "/api/permissions/1", body: .object(["decision": .string("deny")]))
        s.items = [Item(id: "1", title: "Dev wants to use Bash", agentName: "Dev", urgency: 3, link: .needsYou, summary: "Run the migration",
                        approve: .needsYouItem("1"), deny: deny),
                   Item(id: "2", title: "QA asked you something", agentName: "QA", urgency: 2, link: .needsYou, summary: "Ship it on Friday?")]
        s.working = ["Dev", "QA"]
        s.weekSpend = 12.4
        return s
    }
}
