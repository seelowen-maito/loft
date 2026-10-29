import SwiftUI

/// A Loft issue's thread: comments, agents' notes and status changes, oldest first, with a box to comment.
/// On a mirrored issue each comment says where it stands with GitHub; one waiting for approval has Approve and
/// Keep in Loft right there (the same as its Needs you item).
struct TaskThreadSection: View {
    @Environment(LoftStore.self) private var store
    let task: HBTask
    @State private var comments: [TaskComment] = []
    @State private var draft = ""
    /// On a mirrored issue: whether a new comment may go to GitHub (after approval, as the setting says).
    @State private var toGitHub = true
    @State private var busy = false
    @State private var error: APIError?

    var body: some View {
        Section {
            if comments.isEmpty {
                Text("No comments yet.").foregroundStyle(.secondary)
            }
            ForEach(comments) { c in
                if c.kind == "status" {
                    Label(c.text, systemImage: "arrow.triangle.2.circlepath")
                        .font(.footnote).foregroundStyle(.secondary)
                } else {
                    entry(c)
                }
            }
            if task.github != nil {
                Toggle("Post on GitHub", isOn: $toGitHub)
                    .font(.subheadline)
                    .accessibilityHint("Off keeps the comment in Loft.")
                    .accessibilityIdentifier("threadPostOnGitHub")
            }
            HStack(alignment: .bottom) {
                // No promise about GitHub here: the server's approval setting decides whether a comment waits or goes.
                TextField("Comment…", text: $draft, axis: .vertical)
                    .lineLimit(1...6)
                    .accessibilityIdentifier("threadField")
                Button {
                    let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                    act {
                        try await store.addTaskComment(task.id, text: text, github: task.github == nil ? nil : toGitHub)
                        draft = ""
                    }
                } label: { Image(systemName: "arrow.up.circle.fill").font(.title2) }
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || busy || !store.canWrite)
                .accessibilityLabel("Send comment")
                .accessibilityIdentifier("threadSend")
            }
        } header: {
            Text("Thread")
        }
        .task(id: task.updatedAt) { await load() }
        .errorAlert($error)
    }

    private func entry(_ c: TaskComment) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(author(c)).font(.subheadline.weight(.semibold))
                if c.kind == "note" { Text("Note").font(.caption).foregroundStyle(.secondary) }
                Spacer()
                RelativeTime(date: c.createdAt).font(.caption).foregroundStyle(.secondary)
            }
            Text(c.text).font(.subheadline)
                .foregroundStyle(c.kind == "note" ? .secondary : .primary)
            if let g = c.githubState { github(g, c) }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("threadEntry-\(c.id)")
    }

    @ViewBuilder private func github(_ state: String, _ c: TaskComment) -> some View {
        switch state {
        case "pending":
            Label("Not on GitHub yet", systemImage: "clock").font(.caption.weight(.semibold)).foregroundStyle(Theme.attentionInk)
            HStack {
                Button("Approve") { act { try await store.approveComment(c) } }
                    .buttonStyle(.borderedProminent).accessibilityIdentifier("threadApprove-\(c.id)")
                Button("Keep in Loft") { act { try await store.keepComment(c) } }
                    .buttonStyle(.bordered).accessibilityIdentifier("threadKeep-\(c.id)")
            }
            .controlSize(.small)
            .disabled(busy || !store.canWrite)
        case "posted": Label("On GitHub", systemImage: "checkmark.circle").font(.caption).foregroundStyle(.secondary)
        case "kept": Label("Kept in Loft", systemImage: "lock").font(.caption).foregroundStyle(.secondary)
        case "from_github": Label("From GitHub", systemImage: "arrow.down.circle").font(.caption).foregroundStyle(.secondary)
        default: EmptyView()
        }
    }

    private func author(_ c: TaskComment) -> String {
        switch c.author {
        case "user": "You"
        case "github": c.authorName.map { "@\($0)" } ?? "GitHub"
        default: store.agent(c.author)?.name ?? c.authorName ?? c.author
        }
    }

    private func load() async {
        if let c = try? await store.taskComments(task.id) { comments = c }
    }

    /// Runs a change, then reloads the thread so it shows what the server now has.
    private func act(_ f: @escaping () async throws -> Void) {
        busy = true
        Task {
            do { try await f() } catch { self.error = APIError.from(error) }
            await load()
            busy = false
        }
    }
}
