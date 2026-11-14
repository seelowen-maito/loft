import SwiftUI
import WidgetKit

struct UsageEntry: TimelineEntry {
    let date: Date
    let snapshot: UsageWidgetSnapshot
}

struct UsageProvider: TimelineProvider {
    func placeholder(in context: Context) -> UsageEntry { UsageEntry(date: .now, snapshot: .sample) }

    func getSnapshot(in context: Context, completion: @escaping (UsageEntry) -> Void) {
        if context.isPreview { return completion(placeholder(in: context)) }
        Task { completion(UsageEntry(date: .now, snapshot: await UsageWidgetSnapshot.load())) }
    }

    /// Reads Loft again every 15 minutes (the system decides when it really runs); the app reloads it after a sync.
    func getTimeline(in context: Context, completion: @escaping (Timeline<UsageEntry>) -> Void) {
        Task {
            let entry = UsageEntry(date: .now, snapshot: await UsageWidgetSnapshot.load())
            completion(Timeline(entries: [entry], policy: .after(.now.addingTimeInterval(15 * 60))))
        }
    }
}

/// Claude plan usage and the Loft Mac's health (#489): small (session and week), medium (also RAM and temperature)
/// and, on the iPhone, a Lock Screen gauge of the session. A tap opens the Overview.
public struct UsageWidget: Widget {
    public init() {}

    public var body: some WidgetConfiguration {
        StaticConfiguration(kind: "loft.usage", provider: UsageProvider()) { entry in
            UsageEntryView(entry: entry)
                .containerBackground(.background, for: .widget)
                .widgetURL(DeepLink.overview.url)
        }
        .configurationDisplayName("Usage")
        .description("Your Claude session and week, and the Loft Mac's memory and temperature.")
        #if os(iOS)
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryCircular])
        #else
        .supportedFamilies([.systemSmall, .systemMedium])
        #endif
    }
}

private struct UsageEntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: UsageEntry

    var body: some View {
        switch family {
        case .accessoryCircular: UsageGaugeView(entry.snapshot)
        case .systemSmall: UsageWidgetView(entry.snapshot, size: .small)
        default: UsageWidgetView(entry.snapshot, size: .medium)
        }
    }
}

extension UsageWidgetSnapshot {
    /// What the gallery and the placeholder show.
    static var sample: UsageWidgetSnapshot {
        var s = UsageWidgetSnapshot(phase: .ready)
        s.session = Meter(percent: 42, resetsAt: .now.addingTimeInterval(2 * 3600 + 900))
        s.week = Meter(percent: 18, resetsAt: .now.addingTimeInterval(4 * 86400))
        s.memUsedBytes = 21 << 30
        s.memTotalBytes = 32 << 30
        s.memLevel = "nominal"
        s.cpuTempC = 64
        s.tempLevel = "nominal"
        s.weekSpend = 12.4
        s.takenAt = .now
        return s
    }
}

/// The colours Loft uses for a level, as the web: usage amber at 75 %, red at 90 %; memory high / critical; CPU warm / hot.
enum UsageTone {
    static func usage(_ percent: Double?) -> Color {
        guard let p = percent else { return .secondary }
        return p >= 90 ? Theme.failure : p >= 75 ? Theme.attention : Theme.accent
    }

    static func level(_ level: String?) -> Color {
        switch level {
        case "hot", "critical": Theme.failure
        case "warm", "high": Theme.attention
        default: .primary
        }
    }
}

/// The Usage widget's contents, small and medium.
public struct UsageWidgetView: View {
    public enum Size { case small, medium }

    let snapshot: UsageWidgetSnapshot
    let size: Size

    public init(_ snapshot: UsageWidgetSnapshot, size: Size) {
        self.snapshot = snapshot
        self.size = size
    }

    public var body: some View {
        switch snapshot.phase {
        case .notConnected: LoftWidgetView(.notConnected, size: .small)
        case .unreachable where snapshot.takenAt == nil: LoftWidgetView(.unreachable, size: .small)
        default: reading
        }
    }

    private var reading: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    meter("Session", snapshot.session)
                    meter("Week", snapshot.week)
                }
                .frame(maxWidth: .infinity)
                if size == .medium {
                    VStack(alignment: .leading, spacing: 8) {
                        stat("RAM", snapshot.memoryText, symbol: "memorychip", tone: UsageTone.level(snapshot.memLevel))
                        stat("CPU", snapshot.tempText, symbol: "thermometer.medium", tone: UsageTone.level(snapshot.tempLevel))
                        if let spend = snapshot.spendText {
                            Text("\(spend) this week").font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    .frame(width: 112, alignment: .leading)
                }
            }
            Spacer(minLength: 0)
            footer
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func meter(_ title: String, _ m: UsageWidgetSnapshot.Meter?) -> some View {
        let fraction = min(max((m?.percent ?? 0) / 100, 0), 1)
        return VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline) {
                Text(title).font(.caption.weight(.semibold))
                Spacer(minLength: 4)
                Text(UsageWidgetSnapshot.percentText(m)).font(.caption.monospacedDigit().weight(.semibold))
                    .foregroundStyle(m == nil ? Color.secondary : UsageTone.usage(m?.percent))
            }
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.secondary.opacity(0.2))
                    Capsule().fill(UsageTone.usage(m?.percent)).frame(width: g.size.width * fraction)
                }
            }
            .frame(height: 6)
            Text(m.map { UsageWidgetSnapshot.resetText($0.resetsAt, now: .now) } ?? "No reading")
                .font(.caption2).foregroundStyle(.secondary).lineLimit(1).minimumScaleFactor(0.8)
        }
    }

    private func stat(_ title: String, _ value: String, symbol: String, tone: Color) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Label(title, systemImage: symbol).font(.caption2).foregroundStyle(.secondary)
            Text(value).font(.subheadline.monospacedDigit().weight(.semibold)).foregroundStyle(value == "—" ? Color.secondary : tone)
                .lineLimit(1).minimumScaleFactor(0.8)
        }
    }

    /// Offline: the reading's time, so an old number never passes for a live one.
    @ViewBuilder private var footer: some View {
        if snapshot.phase == .unreachable, let at = snapshot.takenAt {
            Label { Text("Offline · from \(Text(at, style: .time))") } icon: { Image(systemName: "bolt.horizontal.circle") }
                .font(.caption2).foregroundStyle(Theme.attention).lineLimit(1).minimumScaleFactor(0.8)
        }
    }
}

/// The Lock Screen's round gauge: the session's percent.
public struct UsageGaugeView: View {
    let snapshot: UsageWidgetSnapshot

    public init(_ snapshot: UsageWidgetSnapshot) { self.snapshot = snapshot }

    public var body: some View {
        let session = snapshot.phase == .notConnected ? nil : snapshot.session
        Gauge(value: min(max((session?.percent ?? 0) / 100, 0), 1)) {
            Image(systemName: snapshot.phase == .unreachable ? "bolt.horizontal" : "gauge.medium")
        } currentValueLabel: {
            Text(session.map { "\(Int($0.percent.rounded()))" } ?? "—")
        }
        .gaugeStyle(.accessoryCircular)
    }
}
