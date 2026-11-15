import LoftKit
import SwiftUI

/// Needs you: everything waiting on you, most urgent first (the server's order), PRs grouped by project. Requests run
/// on a click; replies go through a sheet you send yourself.
struct NeedsYouScreen: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav

    var body: some View {
        ScrollViewReader { proxy in
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                if let items = store.needsYou?.items, !items.isEmpty {
                    ForEach(NeedsYouSection.build(items)) { section in
                        switch section {
                        case .item(let i):
                            NeedsYouRow(item: i, showsText: i.id == nav.needsYouFocus)
                                .overlay { if i.id == nav.needsYouFocus { RoundedRectangle(cornerRadius: 10).stroke(Theme.accent, lineWidth: 2) } }
                                .id(i.id)
                        case .project(let name, let prs):
                            DisclosureGroup {
                                ForEach(prs) { NeedsYouRow(item: $0) }
                            } label: {
                                Label("\(name) · \(prs.count)", systemImage: "folder").font(.headline)
                            }
                            .accessibilityIdentifier("needsYouProject-\(name)")
                        }
                    }
                }
            }
            .frame(maxWidth: 820)
            .padding(20)
            .frame(maxWidth: .infinity)
        }
        .onChange(of: nav.needsYouFocus.map { id in store.needsYou?.items.contains { $0.id == id } == true } == true, initial: true) { _, there in
            if there, let id = nav.needsYouFocus { proxy.scrollTo(id, anchor: .center) }
        }
        }
        .overlay {
            if store.needsYou?.items.isEmpty != false {
                ContentUnavailableView("Nothing needs you", systemImage: "checkmark.circle",
                                       description: Text("Agents show up here when they need an answer or an approval."))
            }
        }
        .overlay(alignment: .top) { NeedsYouNotice().padding(.top, 8) }
        .navigationTitle("Needs you")
        .toolbar {
            Button { Task { await store.refreshNeedsYou() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                .keyboardShortcut("r")
        }
        .task { await store.refreshNeedsYou() }
    }
}

/// One item: what it is, why it's here, and what you can do.
struct NeedsYouRow: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @Environment(\.openURL) private var openURL
    let item: NeedsYouItem
    /// The whole text, uncut (the full command, for a permission): the card the Needs you widget's Approve opens.
    var showsText = false
    /// The on-device model's one-line summary and suggested replies (they only fill the reply sheet).
    @State private var assist: NeedsYouAssist?
    @State private var answering: String?
    @State private var busy = false
    @State private var error: String?

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
                Text(item.title).font(.headline).lineLimit(2)
                Spacer(minLength: 0)
                if let age = item.createdAt { Text(age.shortAgo).font(.caption).foregroundStyle(.secondary) }
            }
            if let s = assist?.summary {
                Label(s, systemImage: "sparkles").accessibilityLabel("Summary: \(s)")
            }
            Text(item.text.isEmpty ? item.reason : item.text)
                .foregroundStyle(.secondary)
                .lineLimit(showsText ? nil : 4)
                .font(showsText ? .body.monospaced() : nil)
                .textSelection(.enabled)
            // Only when there's a real choice: a lone suggestion read as a command.
            if let replies = assist?.replies, replies.count >= 2, item.answerAction != nil {
                HStack(spacing: 6) {
                    ForEach(replies, id: \.self) { r in
                        Button(r) { answering = r }.buttonStyle(.bordered).controlSize(.small).help("Fills in the reply; nothing is sent")
                    }
                }
                .accessibilityIdentifier("needsYouSuggestions")
            }
            HStack(spacing: 6) {
                ForEach(Array(item.actions.filter { $0.kind != "dismiss" }.enumerated()), id: \.offset) { _, a in action(a) }
                Spacer(minLength: 0)
                if let d = item.actions.first(where: { $0.kind == "dismiss" }) {
                    Button(d.label) { run(d) }.buttonStyle(.borderless).foregroundStyle(.secondary).disabled(busy)
                }
            }
            .controlSize(.small)
            if let error { Text(error).font(.caption).foregroundStyle(Theme.failureInk) }
        }
        .padding(12)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("needsYou-\(item.id)")
        .task(id: item.id) {
            guard item.answerAction != nil || item.text.count > item.reason.count + 40 else { return }
            assist = await OnDeviceFeatures.needsYouAssist(item)
        }
        .sheet(item: Binding(get: { answering.map(AnswerDraft.init) }, set: { answering = $0?.text })) { draft in
            NeedsYouAnswerSheet(item: item, text: draft.text, sendLabel: item.type == "comment_approval" ? "Approve" : "Send") { text, agentId in
                if let a = item.answerAction { reply(a, text: text, agentId: agentId) }
            }
        }
    }

    @ViewBuilder private func action(_ a: NeedsYouAction) -> some View {
        switch a.kind {
        case "open":
            if let t = a.target {
                Button(a.label) {
                    if let s = t.url {
                        if let u = PushDestination.needsYouLink(s, loftHost: store.api?.baseURL.host(), tailnetHost: store.info?.host) { openURL(u) }
                    } else if t.screen == "task" {
                        nav.section = .issues
                    } else if let id = t.agentId {
                        nav.openChat(id)
                    }
                }
            }
        case "answer":
            Button(a.label) { answering = a.draft }
                .buttonStyle(.accent)
                .accessibilityIdentifier("needsYouAnswer")
        case "deny":
            Button(a.label, role: .destructive) { run(a) }.disabled(busy)
        default:
            if a.kind == "approve" || a.kind == "merge" {
                Button(a.label) { run(a) }.buttonStyle(.accent).disabled(busy || !store.canWrite)
            } else {
                Button(a.label) { run(a) }.disabled(busy || !store.canWrite)
            }
        }
    }

    /// A reply on an issue says where it went: to an agent, or kept as a note only (and then the item goes too).
    private func reply(_ a: NeedsYouAction, text: String, agentId: String?) {
        guard a.isIssueReply else { return run(a, text: text) }
        let to = item.agent?.name ?? store.agent(agentId)?.name
        Task {
            busy = true
            defer { busy = false }
            do {
                guard try await store.perform(a, text: text, agentId: agentId) else { store.needsYouNotice = "Already done"; return }
                if let to {
                    store.needsYouNotice = "Sent to \(to)"
                } else {
                    if let d = item.actions.first(where: { $0.kind == "dismiss" }) { _ = try await store.perform(d) }
                    store.needsYouNotice = "Saved as a note only"
                }
            } catch { self.error = APIError.from(error).title }
        }
    }

    private func run(_ a: NeedsYouAction, text: String? = nil) {
        Task {
            busy = true
            defer { busy = false }
            do {
                error = nil
                if try await !store.perform(a, text: text) { store.needsYouNotice = "Already done" }
                else if a.kind == "answer" { store.needsYouNotice = "Sent" }
            } catch { self.error = APIError.from(error).title }
        }
    }
}

/// "Already done", "Sent to …": a short note at the top that goes after two seconds.
private struct NeedsYouNotice: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        if let n = store.needsYouNotice {
            Label(n, systemImage: "checkmark.circle")
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(.regularMaterial, in: Capsule())
                .accessibilityIdentifier("needsYouNotice")
                .task(id: n) {
                    guard (try? await Task.sleep(for: .seconds(2))) != nil, store.needsYouNotice == n else { return }
                    store.needsYouNotice = nil
                }
        }
    }
}

private struct AnswerDraft: Identifiable {
    let text: String
    var id: String { "draft" }
}

/// Write (or edit) a reply; nothing goes until Send.
private struct NeedsYouAnswerSheet: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let item: NeedsYouItem
    @State var text: String
    var sendLabel = "Send"
    let send: (String, String?) -> Void
    /// On an issue with nobody on it: who gets the reply. None keeps it as a note.
    @State private var agentId: String?
    private var asksForAgent: Bool { item.agent == nil && item.answerAction?.isIssueReply == true }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(item.agent?.name ?? "Reply").font(.headline)
            ScrollView { Text(item.text).foregroundStyle(.secondary).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                .frame(maxHeight: 120)
            Text(item.type == "comment_approval" ? "Comment to post on GitHub" : "Your reply").font(.subheadline)
            TextEditor(text: $text)
                .font(.body)
                .frame(minHeight: 100)
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
                .accessibilityIdentifier("needsYouAnswerField")
            if asksForAgent {
                Picker("Send to", selection: $agentId) {
                    Text("Nobody").tag(String?.none)
                    ForEach(store.agents) { a in Text(a.name).tag(Optional(a.id)) }
                }
                Text(agentId == nil ? "Nobody is on this issue: the reply is kept as a note only." : "The agent takes the issue and gets your reply.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(sendLabel) { send(text, asksForAgent ? agentId : nil); dismiss() }
                    .keyboardShortcut(.return, modifiers: .command)
                    .buttonStyle(.accent)
                    .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("needsYouAnswerSend")
            }
        }
        .padding()
        .frame(width: 480)
    }
}
