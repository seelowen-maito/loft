import Charts
import SwiftUI

extension Theme {
    /// The two run series, validated as a categorical pair (CVD and contrast) on the light and dark card surfaces.
    static let scheduled = Color(red: 0x5B / 255, green: 0x6C / 255, blue: 0xF0 / 255)
    static let requested = Color(red: 0x14 / 255, green: 0x93 / 255, blue: 0x9F / 255)
}

// MARK: - Plan usage

/// A usage level, tinted like the web meter: normal, amber at 75%, red at 90%.
enum UsageLevel {
    case normal, high, critical

    init(_ percent: Double) {
        self = percent >= 90 ? .critical : percent >= 75 ? .high : .normal
    }

    var color: Color {
        switch self {
        case .normal: Theme.accent
        case .high: Theme.attention
        case .critical: Theme.failure
        }
    }

    /// Status is never color alone: high and critical also get a symbol.
    var symbol: String? {
        switch self {
        case .normal: nil
        case .high: "exclamationmark.circle.fill"
        case .critical: "exclamationmark.triangle.fill"
        }
    }
}

/// One plan window (session or week) as a labelled bar.
struct UsageMeter: View {
    let title: String
    let window: PlanUsage.Window

    var body: some View {
        let level = UsageLevel(window.percent)
        let fraction = min(max(window.percent / 100, 0), 1)
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(title).font(.subheadline.weight(.semibold))
                Spacer()
                if let s = level.symbol { Image(systemName: s).font(.caption).foregroundStyle(level.color) }
                Text("\(Int(window.percent.rounded()))%")
                    .font(.subheadline.monospacedDigit().weight(.semibold))
                    .contentTransition(.numericText())
            }
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.secondary.opacity(0.18))
                    Capsule().fill(level.color).frame(width: max(g.size.width * fraction, fraction > 0 ? 6 : 0))
                }
            }
            .frame(height: 8)
            if let at = window.resetsAt {
                TimelineView(.periodic(from: .now, by: 60)) { ctx in
                    Text("Resets in \(Self.until(at, from: ctx.date))")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue("\(Int(window.percent.rounded())) percent used" + (window.resetsAt.map { ", resets in \(Self.until($0, from: .now))" } ?? ""))
        .animation(.default, value: window.percent)
    }

    /// "2h 15m", "3d 4h", "12m", "now".
    static func until(_ date: Date, from now: Date) -> String {
        let s = Int(date.timeIntervalSince(now))
        guard s > 59 else { return "under a minute" }
        let d = s / 86_400, h = (s % 86_400) / 3600, m = (s % 3600) / 60
        if d > 0 { return h > 0 ? "\(d)d \(h)h" : "\(d)d" }
        if h > 0 { return m > 0 ? "\(h)h \(m)m" : "\(h)h" }
        return "\(m)m"
    }
}

// MARK: - Context

/// "ctx 182k": an agent's context size, amber past the 200k window like the web UI.
struct ContextChip: View {
    let tokens: Int

    private var over: Bool { tokens >= ContextWindow.warnAt }

    var body: some View {
        HStack(spacing: 3) {
            if over { Image(systemName: "exclamationmark.triangle.fill") }
            Text("ctx \(Self.short(tokens))").monospacedDigit()
        }
        .font(.caption2.weight(.semibold))
        .fixedSize()
        .padding(.horizontal, 6)
        .padding(.vertical, 2)
        .foregroundStyle(over ? Theme.attention : .secondary)
        .background((over ? Theme.attention : Color.secondary).opacity(0.15), in: Capsule())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Context \(Self.short(tokens)) tokens" + (over ? ", over the 200k window" : ""))
        .accessibilityIdentifier(over ? "contextChipOver" : "contextChip")
    }

    static func short(_ n: Int) -> String {
        n >= 1_000_000 ? String(format: "%.1fM", Double(n) / 1_000_000)
            : n >= 1000 ? "\(Int((Double(n) / 1000).rounded()))k" : "\(n)"
    }
}

// MARK: - Activity

/// Runs per day for the last N days, oldest first, split into scheduled wake-ups and ones that were asked for.
struct ActivityDay: Identifiable {
    let date: Date
    let scheduled: Int
    let requested: Int
    var id: Date { date }
    var total: Int { scheduled + requested }

    static func build(auto: [Int], user: [Int], calendar: Calendar = .current, today: Date = .now) -> [ActivityDay] {
        let n = max(auto.count, user.count)
        let start = calendar.startOfDay(for: today)
        return (0..<n).map { i in
            let date = calendar.date(byAdding: .day, value: i - (n - 1), to: start) ?? start
            return ActivityDay(date: date,
                               scheduled: auto.indices.contains(i) ? auto[i] : 0,
                               requested: user.indices.contains(i) ? user[i] : 0)
        }
    }
}

/// Stacked bars of runs per day with a legend and a per-day tooltip on touch.
struct ActivityChart: View {
    let days: [ActivityDay]
    var height: CGFloat = 150
    @State private var selected: Date?

    private var selectedDay: ActivityDay? {
        guard let selected else { return nil }
        return days.first { Calendar.current.isDate($0.date, inSameDayAs: selected) }
    }

    var body: some View {
        Chart {
            ForEach(days) { d in
                BarMark(x: .value("Day", d.date, unit: .day), y: .value("Runs", d.scheduled))
                    .foregroundStyle(by: .value("Kind", "Scheduled"))
                BarMark(x: .value("Day", d.date, unit: .day), y: .value("Runs", d.requested))
                    .foregroundStyle(by: .value("Kind", "Requested"))
            }
            if let d = selectedDay {
                RuleMark(x: .value("Day", d.date, unit: .day))
                    .foregroundStyle(Color.secondary.opacity(0.25))
                    .zIndex(-1)
                    .annotation(position: .top, overflowResolution: .init(x: .fit(to: .chart), y: .disabled)) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(d.date.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated)))
                                .font(.caption2.weight(.semibold))
                            Text("\(d.scheduled) scheduled · \(d.requested) requested").font(.caption2)
                        }
                        .padding(6)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    }
            }
        }
        .chartForegroundStyleScale(["Scheduled": Theme.scheduled, "Requested": Theme.requested])
        .chartLegend(position: .top, alignment: .leading, spacing: 8)
        .chartXAxis {
            AxisMarks(values: .stride(by: .day, count: 7)) { _ in
                AxisValueLabel(format: .dateTime.day().month(.abbreviated))
            }
        }
        .chartYAxis {
            AxisMarks(position: .trailing, values: .automatic(desiredCount: 3)) { _ in
                AxisGridLine(stroke: StrokeStyle(lineWidth: 0.5)).foregroundStyle(Color.secondary.opacity(0.25))
                AxisValueLabel()
            }
        }
        .chartXSelection(value: $selected)
        .frame(height: height)
        .accessibilityLabel("Runs per day, last \(days.count) days")
    }
}

/// A tiny per-day bar strip for an agent: the shape of its last two weeks, no axes.
struct Sparkline: View {
    let days: [ActivityDay]

    var body: some View {
        Chart(days) { d in
            BarMark(x: .value("Day", d.date, unit: .day), y: .value("Runs", d.total))
                .foregroundStyle(d.total > 0 ? Theme.scheduled : Color.secondary.opacity(0.3))
                .cornerRadius(2)
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .chartYScale(domain: 0...max(1, days.map(\.total).max() ?? 1))
        .frame(height: 36)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(days.reduce(0) { $0 + $1.total }) runs in the last \(days.count) days")
    }
}
