import SwiftUI

enum Theme {
    static let accent = Color.accentColor
    static let working = Color(red: 0.30, green: 0.82, blue: 0.52)
    static let attention = Color(red: 1.00, green: 0.72, blue: 0.25)
    static let failure = Color(red: 1.00, green: 0.42, blue: 0.40)
    /// Text in those colours on a pale tint of themselves (risk labels): the same in dark mode, darker in light
    /// mode, where the bright amber and red read faintly on white (about 5:1 instead of 1.6:1 and 2.5:1).
    static let attentionInk = ink(light: UIColor(red: 0.60, green: 0.36, blue: 0.00, alpha: 1), dark: attention)
    static let failureInk = ink(light: UIColor(red: 0.76, green: 0.18, blue: 0.16, alpha: 1), dark: failure)
    static let neutral = Color.secondary
    static let card = Color(uiColor: .secondarySystemGroupedBackground)
    static let background = Color(uiColor: .systemGroupedBackground)

    static let radius: CGFloat = 14
    static let gutter: CGFloat = 16

    private static func ink(light: UIColor, dark: Color) -> Color {
        Color(uiColor: UIColor { $0.userInterfaceStyle == .dark ? UIColor(dark) : light })
    }
}

extension AgentState {
    var color: Color {
        switch self {
        case .working: Theme.working
        case .waitingApproval: Theme.attention
        case .failed: Theme.failure
        case .idle, .unknown: Theme.neutral
        }
    }
}

extension TaskState {
    var color: Color {
        switch self {
        case .inProgress: Theme.working
        case .blocked, .waitingApproval: Theme.attention
        case .failed: Theme.failure
        case .done: Theme.accent
        case .inbox, .ready, .cancelled, .unknown: Theme.neutral
        }
    }
}

extension ConnectionState {
    var color: Color {
        switch self {
        case .connected: Theme.working
        case .polling, .connecting: Theme.attention
        case .reconnecting(let attempt, _): attempt <= 1 ? Theme.attention : Theme.failure
        case .notConfigured: Theme.neutral
        }
    }
}

/// Icon + text status, so state never depends on color alone.
struct StatusBadge: View {
    let text: String
    let symbol: String
    let color: Color
    var compact = false

    var body: some View {
        // An HStack rather than a Label: Label's default icon gap is wide for a chip this small.
        HStack(spacing: compact ? 3 : 4) {
            Image(systemName: symbol)
            Text(text)
        }
        .font(compact ? .caption2.weight(.semibold) : .caption.weight(.semibold))
        .fixedSize()
        .foregroundStyle(color)
        .padding(.horizontal, compact ? 6 : 8)
        .padding(.vertical, compact ? 2 : 4)
        .background(color.opacity(0.14), in: Capsule())
        .accessibilityElement(children: .combine)
    }
}

extension StatusBadge {
    init(_ s: AgentState, compact: Bool = false) {
        self.init(text: compact ? s.shortLabel : s.label, symbol: s.symbol, color: s.color, compact: compact)
    }

    init(_ s: TaskState, compact: Bool = false) {
        self.init(text: s.label, symbol: s.symbol, color: s.color, compact: compact)
    }
}

/// A round monogram. Loft avatars are SVG stickers, which iOS can't draw
/// natively, so the app uses a stable color per agent instead.
struct AgentAvatar: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.displayScale) private var displayScale
    let name: String
    let id: String
    var size: CGFloat = 36
    var state: AgentState?

    private static let palette: [Color] = [
        Color(red: 0.49, green: 0.55, blue: 1.0), Color(red: 0.96, green: 0.55, blue: 0.36),
        Color(red: 0.33, green: 0.78, blue: 0.70), Color(red: 0.85, green: 0.47, blue: 0.85),
        Color(red: 0.95, green: 0.75, blue: 0.30), Color(red: 0.45, green: 0.72, blue: 0.95),
        Color(red: 0.90, green: 0.45, blue: 0.55), Color(red: 0.55, green: 0.80, blue: 0.40),
    ]

    private var color: Color {
        let h = id.unicodeScalars.reduce(0) { ($0 &* 31 &+ Int($1.value)) & 0xFFFF }
        return Self.palette[h % Self.palette.count]
    }

    private var initials: String {
        let words = name.split(whereSeparator: { $0 == " " || $0 == "-" || $0 == "_" })
        let letters = words.prefix(2).compactMap(\.first)
        return String(letters).uppercased()
    }

    var body: some View {
        face
            .frame(width: size, height: size)
            .overlay(alignment: .bottomTrailing) {
                if let state, state != .unknown {
                    Circle()
                        .fill(state.color)
                        .frame(width: size * 0.3, height: size * 0.3)
                        .overlay(Circle().stroke(Theme.card, lineWidth: 2))
                }
            }
            .accessibilityHidden(true)
    }

    /// The agent's sticker when it has one that renders, else initials on a colored circle.
    @ViewBuilder private var face: some View {
        // The sticker canvas has 14/128 of padding for its border and shadow; draw it a bit larger
        // so the character fills about the same circle the initials did.
        if let svg = store.agents.first(where: { $0.id == id })?.avatar?.svg,
           StickerRenderer.image(svg: svg, points: size * 1.25, scale: displayScale) != nil {
            StickerImage(svg: svg, size: size * 1.25)
                .frame(width: size, height: size)
                .accessibilityHidden(true)
        } else {
            Circle()
                .fill(color.gradient)
                .overlay {
                    Text(initials)
                        .font(.system(size: size * 0.38, weight: .bold, design: .rounded))
                        .foregroundStyle(.white)
                }
        }
    }

}

struct Card<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) { content }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: Theme.radius, style: .continuous))
    }
}

struct SectionHeader: View {
    let title: String
    var trailing: AnyView?

    init(_ title: String, trailing: AnyView? = nil) {
        self.title = title
        self.trailing = trailing
    }

    var body: some View {
        AdaptiveStack {
            Text(title).font(.headline)
            Spacer(minLength: 0)
            trailing
        }
        .padding(.top, 8)
    }
}

/// Explains why nothing is shown and, where it helps, offers the next step.
struct EmptyState: View {
    let symbol: String
    let title: String
    let message: String
    var actionTitle: String?
    var action: (() -> Void)?

    var body: some View {
        ContentUnavailableView {
            Label(title, systemImage: symbol)
        } description: {
            Text(message)
        } actions: {
            if let actionTitle, let action {
                Button(actionTitle, action: action).buttonStyle(.borderedProminent)
            }
        }
    }
}

/// Shown above content that came from the cache rather than a live connection.
struct StaleBanner: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        if !store.isLive, store.connection != .notConfigured {
            HStack(spacing: 8) {
                Image(systemName: store.connection == .polling ? "arrow.triangle.2.circlepath" : "wifi.slash")
                VStack(alignment: .leading, spacing: 1) {
                    Text(store.connection.label).font(.footnote.weight(.semibold))
                    if let sync = store.lastSync {
                        Text(store.connection == .polling ? "Refreshes every few seconds · last \(sync.formatted(date: .omitted, time: .shortened))"
                             : "Showing data from \(sync.formatted(date: .abbreviated, time: .shortened))")
                            .font(.caption)
                    } else {
                        Text("Nothing loaded yet").font(.caption)
                    }
                }
                Spacer()
                if store.connection != .polling {
                    Button("Retry") { store.reconnect() }
                        .font(.footnote.weight(.semibold))
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                }
            }
            .foregroundStyle(store.connection.color)
            .padding(10)
            // Tinted over a solid base: it floats over scrolling chat messages, which showed through the tint alone.
            .background {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Theme.background)
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(store.connection.color.opacity(0.12)))
            }
            .accessibilityIdentifier("staleBanner")
        }
    }
}

/// Text that collapses past a few lines, with a toggle to expand.
struct CollapsibleText: View {
    let text: String
    var lineLimit = 12
    @State private var expanded = false

    private var isLong: Bool { text.count > 700 || text.filter { $0 == "\n" }.count > lineLimit }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(Markdown.render(text))
                .lineLimit(expanded || !isLong ? nil : lineLimit)
                .textSelection(.enabled)
            if isLong {
                Button(expanded ? "Show less" : "Show more") {
                    withAnimation(.snappy) { expanded.toggle() }
                }
                .font(.caption.weight(.semibold))
            }
        }
    }
}

enum Markdown {
    static func render(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)))
            ?? AttributedString(s)
    }
}

extension View {
    /// Runs an action and reports its error in an alert instead of failing silently.
    func errorAlert(_ error: Binding<APIError?>) -> some View {
        alert(error.wrappedValue?.title ?? "", isPresented: Binding(get: { error.wrappedValue != nil }, set: { if !$0 { error.wrappedValue = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(error.wrappedValue?.suggestion ?? "")
        }
    }
}

/// "now", "4m", "3h"… that keeps itself current. A plain `date.shortAgo` is computed once when the row is
/// drawn, so an event that arrived "now" kept saying "now".
struct RelativeTime: View {
    let date: Date

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { _ in
            Text(date.shortAgo)
        }
    }
}

extension LoftStore {
    /// Something has been loaded (live, or from the cache). Until then an empty list means "don't know yet", not "none".
    var hasLoaded: Bool { lastSync != nil }
}

/// What a list shows before anything has loaded: connecting, or why Loft can't be reached.
/// Used instead of "No agents" / "No tasks", which would be wrong while nothing is known.
struct NotLoadedView: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        Group {
            if case .reconnecting(_, let error) = store.connection {
                EmptyState(symbol: "wifi.slash", title: "Can't reach Loft",
                           message: (error.title == "Can't reach Loft" ? "" : "\(error.title). ")
                               + "Check that Loft is running on your Mac and that this iPhone is on your tailnet.",
                           actionTitle: "Retry", action: { store.reconnect() })
            } else {
                ProgressView("Connecting to Loft…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .accessibilityIdentifier("notLoaded")
    }
}

extension ConnectionState {
    var isReconnecting: Bool { if case .reconnecting = self { true } else { false } }
}

/// A row that turns into a column at accessibility text sizes, so labels and buttons don't break mid-word.
struct AdaptiveStack<Content: View>: View {
    @Environment(\.dynamicTypeSize) private var typeSize
    var spacing: CGFloat = 8
    @ViewBuilder var content: Content

    var body: some View {
        let layout = typeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: spacing))
            : AnyLayout(HStackLayout(spacing: spacing))
        layout { content }
    }
}

/// Status chips side by side, or stacked when they don't fit (large text on a narrow screen).
struct ChipRow<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 6) { content }
            VStack(alignment: .leading, spacing: 4) { content }
        }
    }
}

extension View {
    /// `lineLimit(n)`, lifted at accessibility text sizes: there one or two lines hold only a few words,
    /// so truncating would hide most of the text.
    func readableLineLimit(_ n: Int) -> some View { modifier(ReadableLineLimit(limit: n)) }
}

private struct ReadableLineLimit: ViewModifier {
    @Environment(\.dynamicTypeSize) private var typeSize
    let limit: Int

    func body(content: Content) -> some View {
        content.lineLimit(typeSize.isAccessibilitySize ? nil : limit)
    }
}

extension View {
    /// iOS 26's default (soft) scroll edge only fades content under the navigation and tab bars, so
    /// bubbles and cards stay legible behind titles and the status bar. The hard style puts a solid
    /// backing behind the bars once content scrolls under them. Earlier iOS already uses a bar material.
    @ViewBuilder func readableBarEdges() -> some View {
        if #available(iOS 26, *) {
            scrollEdgeEffectStyle(.hard, for: [.top, .bottom])
        } else {
            self
        }
    }
}

extension LoftStore {
    /// The address the app is talking to, as host[:port] (the port only when it isn't the scheme's default).
    var serverAddress: String? {
        guard let url = api?.baseURL, let host = url.host() else { return nil }
        return url.port.map { "\(host):\($0)" } ?? host
    }
}

extension Binding {
    /// The same binding, also calling `action` after each change.
    func onChange(_ action: @escaping () -> Void) -> Binding<Value> {
        Binding(get: { wrappedValue }, set: { wrappedValue = $0; action() })
    }
}
