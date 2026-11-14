import SwiftUI

/// The widgets' contents, small and medium, shared by the Mac desktop and the iPhone home screen. They paint no
/// background: the widget adds its own (`containerBackground`).
public struct LoftWidgetView: View {
    public enum Size { case small, medium }

    let snapshot: WidgetSnapshot
    let size: Size

    public init(_ snapshot: WidgetSnapshot, size: Size) {
        self.snapshot = snapshot
        self.size = size
    }

    public var body: some View {
        switch snapshot.phase {
        case .notConnected: message("Not connected", "Open Loft and connect it.", symbol: "link.badge.plus")
        case .unreachable: message("Can't reach Loft", "Check that Loft is running and reachable.", symbol: "bolt.horizontal.circle")
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
            VStack(alignment: .leading, spacing: 4) {
                needsHeader
                Spacer(minLength: 0)
                workingLine
                if let spend = snapshot.spendText { Text("\(spend) this week").font(.caption2).foregroundStyle(.secondary) }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        } else {
            HStack(alignment: .top, spacing: 14) {
                VStack(alignment: .leading, spacing: 6) {
                    needsHeader.widgetLink(DeepLink.needsYou)
                    ForEach(snapshot.items.prefix(WidgetSnapshot.overviewLimit)) { item in
                        VStack(alignment: .leading, spacing: 0) {
                            Text(item.title).font(.caption).lineLimit(1)
                            if let agent = item.agentName { Text(agent).font(.caption2).foregroundStyle(.secondary).lineLimit(1) }
                        }
                        .widgetLink(item.link)
                    }
                    if snapshot.items.isEmpty { Text("Nothing is waiting on you.").font(.caption).foregroundStyle(.secondary).widgetLink(DeepLink.needsYou) }
                    Spacer(minLength: 0)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Working now").font(.caption.weight(.semibold)).foregroundStyle(.secondary).textCase(.uppercase)
                    ForEach(snapshot.working.prefix(3), id: \.self) { Text($0).font(.caption).lineLimit(1) }
                    if snapshot.working.isEmpty { Text("Nobody").font(.caption).foregroundStyle(.secondary) }
                    Spacer(minLength: 0)
                    if let spend = snapshot.spendText {
                        Text(spend).font(.title3.weight(.semibold))
                        Text("this week").font(.caption2).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .widgetLink(DeepLink.agents)
            }
        }
    }

    private var needsHeader: some View {
        let count = Text("\(snapshot.needsCount)").font(.system(size: size == .small ? 40 : 30, weight: .bold, design: .rounded))
            .foregroundStyle(snapshot.urgent > 0 ? WidgetTheme.attention : WidgetTheme.accent)
        let label = Text("Needs you").font(.subheadline.weight(.semibold))
        return Group {
            if size == .small { VStack(alignment: .leading, spacing: -4) { count; label } }
            else { HStack(alignment: .firstTextBaseline, spacing: 6) { count; label } }
        }
    }

    @ViewBuilder private var workingLine: some View {
        let n = snapshot.working.count
        Label(n == 0 ? "Nobody working" : "\(n) working now", systemImage: "circle.hexagongrid")
            .font(.caption).foregroundStyle(n == 0 ? Color.secondary : WidgetTheme.working)
    }
}

extension EnvironmentValues {
    /// False in screenshots: ImageRenderer draws a `Link` as a "prohibited" sign on the Mac.
    @Entry var widgetLinksTap = true
}

private struct WidgetLink: ViewModifier {
    @Environment(\.widgetLinksTap) private var taps
    let link: DeepLink

    func body(content: Content) -> some View {
        if taps { Link(destination: link.url) { content }.foregroundStyle(.primary) } else { content }
    }
}

extension View {
    /// A tap opens the app at `link`.
    func widgetLink(_ link: DeepLink) -> some View { modifier(WidgetLink(link: link)) }
}

/// Loft's palette (Theme), with the accent fixed to the lime: the widgets draw on a dark tile in either mode.
enum WidgetTheme {
    static let accent = Color(red: 0.78, green: 0.95, blue: 0.43)
    static let working = Theme.working
    static let attention = Theme.attention
}
