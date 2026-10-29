import SwiftUI

struct NeedsYouRoute: Hashable {}

/// The Overview's "Needs you" card: the most urgent few, and the way to the full list.
struct NeedsYouCard: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        if let list = store.needsYou, !list.items.isEmpty {
            SectionHeader("Needs you · \(list.count)", trailing: list.items.count > 3 ? AnyView(
                NavigationLink(value: NeedsYouRoute()) { Text("See all").font(.subheadline) }.accessibilityIdentifier("needsYouSeeAll")
            ) : nil)
            VStack(spacing: 8) {
                ForEach(list.items.prefix(3)) { NeedsYouRow(item: $0) }
            }
            .overlay(alignment: .top) { NeedsYouNotice() }
            .accessibilityIdentifier("needsYouCard")
        }
    }
}

/// Everything waiting on you, most urgent first (the server's order).
struct NeedsYouListView: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        ScrollView {
            VStack(spacing: 8) {
                if let items = store.needsYou?.items, !items.isEmpty {
                    ForEach(NeedsYouSection.build(items)) { section in
                        switch section {
                        case .item(let i): NeedsYouRow(item: i)
                        case .project(let name, let prs): NeedsYouProjectGroup(name: name, items: prs)
                        }
                    }
                } else {
                    EmptyState(symbol: "checkmark.circle", title: "Nothing needs you", message: "Agents will show up here when they need an answer or an approval.")
                        .padding(.top, 40)
                }
            }
            .padding(.horizontal, Theme.gutter)
            .padding(.vertical, 12)
        }
        .background(Theme.background)
        .overlay(alignment: .top) { NeedsYouNotice() }
        .navigationTitle("Needs you")
        .refreshable { await store.refreshNeedsYou() }
        .task { await store.refreshNeedsYou() }
    }
}

/// PRs waiting on you in one GitHub project: a header with the count, which folds the group away.
struct NeedsYouProjectGroup: View {
    let name: String
    let items: [NeedsYouItem]
    @State private var collapsed = false

    var body: some View {
        VStack(spacing: 8) {
            Button { withAnimation(.snappy) { collapsed.toggle() } } label: {
                HStack(spacing: 8) {
                    Image(systemName: "folder").foregroundStyle(.secondary)
                    Text(name).font(.subheadline.weight(.semibold))
                    Text("\(items.count)").font(.caption.weight(.semibold)).monospacedDigit()
                        .padding(.horizontal, 7).padding(.vertical, 2)
                        .background(Color.secondary.opacity(0.15), in: Capsule())
                    Spacer()
                    Image(systemName: "chevron.down").font(.caption).foregroundStyle(.secondary)
                        .rotationEffect(.degrees(collapsed ? -90 : 0))
                }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(name), \(items.count) pull request\(items.count == 1 ? "" : "s")")
            .accessibilityValue(collapsed ? "Collapsed" : "Expanded")
            .accessibilityHint(collapsed ? "Shows them" : "Hides them")
            .accessibilityIdentifier("needsYouProject-\(name)")
            if !collapsed {
                ForEach(items) { NeedsYouRow(item: $0) }
            }
        }
        // A box around the header and its PRs, so the items after it don't read as part of the group.
        .padding(.horizontal, 8)
        .padding(.bottom, collapsed ? 0 : 8)
        .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: Theme.radius + 4, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius + 4, style: .continuous).strokeBorder(Color.secondary.opacity(0.2), lineWidth: 1))
    }
}

/// One item: what it is, why it's here, and what you can do. Requests run on a tap; answers go through a box you
/// send yourself. The on-device model may add a one-line summary and suggested replies (they only fill the box).
struct NeedsYouRow: View {
    @Environment(LoftStore.self) private var store
    let item: NeedsYouItem
    @State private var assist: NeedsYouAssist?
    @State private var answering: String?
    @State private var busy = false
    @State private var error: APIError?

    private var color: Color {
        switch item.urgency {
        case 3: Theme.failure
        case 2: Theme.attention
        case 1: Theme.accent
        default: .secondary
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Circle().fill(color).frame(width: 8, height: 8).accessibilityHidden(true)
                Text(item.title).font(.subheadline.weight(.semibold)).lineLimit(2)
                Spacer(minLength: 0)
                if let age = item.createdAt { Text(age.shortAgo).font(.caption).foregroundStyle(.secondary) }
            }
            if let s = assist?.summary {
                Label(s, systemImage: "sparkles").font(.footnote).accessibilityLabel("Summary: \(s)")
            } else {
                Text(item.reason).font(.footnote).foregroundStyle(.secondary)
            }
            // Suggestions only, styled apart from the actions (dashed, like the quick reply) and only when there's a
            // real choice: a lone "Later" next to Skip read as a duplicate command.
            if let replies = assist?.replies, replies.count >= 2, item.answerAction != nil {
                SuggestedReplies(replies: replies) { answering = $0 }
                    .accessibilityIdentifier("needsYouSuggestions")
            }
            actions
        }
        .padding(12)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .contextMenu {
            if item.canDismiss, let d = item.actions.first(where: { $0.kind == "dismiss" }) {
                Button("Dismiss", systemImage: "eye.slash") { run(d) }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("needsYou-\(item.type)")
        .task(id: item.id) {
            guard item.answerAction != nil || item.text.count > item.reason.count + 40 else { return }
            assist = await OnDeviceFeatures.needsYouAssist(item)
        }
        .sheet(item: Binding(get: { answering.map(AnswerDraft.init) }, set: { answering = $0?.text })) { draft in
            NeedsYouAnswerSheet(item: item, text: draft.text, sendLabel: item.type == "comment_approval" ? "Approve" : "Send") { text in
                guard let a = item.answerAction else { return }
                run(a, text: text)
            }
        }
        .errorAlert($error)
    }

    @ViewBuilder private var actions: some View {
        let visible = item.actions.filter { $0.kind != "dismiss" }
        FlowLayout(spacing: 6) {
            ForEach(Array(visible.enumerated()), id: \.offset) { _, a in
                switch a.kind {
                case "open":
                    if let t = a.target {
                        if let u = t.url.flatMap(URL.init(string:)) {
                            Link(a.label, destination: u).buttonStyle(.bordered).controlSize(.small)
                        } else if t.screen == "task", let id = t.taskId {
                            NavigationLink(a.label, value: TaskRoute(id: id)).buttonStyle(.bordered).controlSize(.small)
                        } else if let id = t.agentId {
                            NavigationLink(a.label, value: ChatRoute(agentId: id)).buttonStyle(.bordered).controlSize(.small)
                        }
                    }
                case "answer":
                    Button(a.label) { answering = a.draft }.buttonStyle(.borderedProminent).controlSize(.small)
                case "deny":
                    Button(a.label, role: .destructive) { run(a) }.buttonStyle(.bordered).controlSize(.small).disabled(busy)
                default:
                    Button(a.label) { run(a) }
                        .buttonStyle(a.kind == "approve" || a.kind == "merge" ? AnyButtonStyle(.borderedProminent) : AnyButtonStyle(.bordered))
                        .controlSize(.small)
                        .disabled(busy || !store.canWrite)
                }
            }
        }
    }

    private func run(_ a: NeedsYouAction, text: String? = nil) {
        Task {
            busy = true
            defer { busy = false }
            do {
                if try await !store.perform(a, text: text) { store.needsYouNotice = "Already done" }
            } catch { self.error = APIError.from(error) }
        }
    }
}

/// "Already done": an action someone (or something) had handled first. Not an error.
private struct NeedsYouNotice: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        if let n = store.needsYouNotice {
            Label(n, systemImage: "checkmark.circle").font(.footnote)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(.regularMaterial, in: Capsule())
                .accessibilityIdentifier("needsYouNotice")
                .task(id: n) {
                    UIAccessibility.post(notification: .announcement, argument: n)
                    try? await Task.sleep(for: .seconds(2))
                    store.needsYouNotice = nil
                }
        }
    }
}

private struct AnswerDraft: Identifiable {
    let text: String
    var id: String { "draft" }
}

/// Write (or edit a suggested) reply; nothing goes until Send.
private struct NeedsYouAnswerSheet: View {
    let item: NeedsYouItem
    @State var text: String
    var sendLabel = "Send"
    let send: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section { Text(item.text).font(.footnote).foregroundStyle(.secondary) }
                Section(item.type == "comment_approval" ? "Comment to post on GitHub" : "Your reply") {
                    TextField("Reply…", text: $text, axis: .vertical).lineLimit(2...8).accessibilityIdentifier("needsYouAnswerField")
                }
            }
            .navigationTitle(item.agent?.name ?? "Reply")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(sendLabel) { send(text); dismiss() }
                        .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("needsYouAnswerSend")
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

/// Picks a button style at run time.
struct AnyButtonStyle: PrimitiveButtonStyle {
    private let make: (Configuration) -> AnyView
    init(_ style: some PrimitiveButtonStyle) { make = { AnyView(style.makeBody(configuration: $0)) } }
    func makeBody(configuration: Configuration) -> some View { make(configuration) }
}
