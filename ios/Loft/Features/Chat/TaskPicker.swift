import SwiftUI

/// "Attach a task" from the composer: search by title or project; open tasks first, the ones waiting on you on
/// top, then the most recently updated (as on the web, #116).
struct TaskPicker: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let pick: (HBTask) -> Void
    @State private var query = ""

    static let order: [TaskState] = [.waitingApproval, .blocked, .failed, .inProgress, .ready, .inbox, .done, .cancelled]

    static func list(_ tasks: [HBTask], query: String) -> [HBTask] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        func rank(_ t: HBTask) -> Int { order.firstIndex(of: t.state) ?? order.count }
        return tasks
            .filter { q.isEmpty || $0.title.lowercased().contains(q) || $0.project.lowercased().contains(q) }
            .sorted { (rank($0), $1.updatedAt) < (rank($1), $0.updatedAt) }
    }

    var body: some View {
        NavigationStack {
            let tasks = Self.list(store.tasks, query: query)
            List {
                Section {
                    ForEach(tasks.prefix(40)) { t in
                        Button { pick(t); dismiss() } label: { row(t) }
                            .accessibilityIdentifier("attachTask-\(t.id)")
                    }
                } footer: {
                    Text("The agent gets the task's title, status and latest note with your message.")
                }
            }
            .overlay { if tasks.isEmpty { ContentUnavailableView.search(text: query) } }
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Title or project")
            .navigationTitle("Attach a task")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
    }

    private func row(_ t: HBTask) -> some View {
        HStack(spacing: 10) {
            Image(systemName: t.state.symbol).foregroundStyle(t.state.color).frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                // Concrete label colours: inside the list's button, .primary and .secondary took the accent tint,
                // so every title read like a link.
                Text(t.title).foregroundStyle(Color(uiColor: .label)).lineLimit(2)
                Text([t.state.label, store.agent(t.agentId)?.name, t.project].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.caption).foregroundStyle(Color(uiColor: .secondaryLabel)).lineLimit(1)
            }
        }
        .frame(minHeight: 44)
        .accessibilityElement(children: .combine)
    }
}

/// A task a message is about: tap to open it.
struct TaskChip: View {
    let task: HBTask

    var body: some View {
        NavigationLink(value: TaskRoute(id: task.id)) {
            HStack(spacing: 4) {
                Image(systemName: task.state.symbol).foregroundStyle(task.state.color)
                Text(task.title).lineLimit(1)
            }
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 8).padding(.vertical, 4)
            .background(Color.secondary.opacity(0.12), in: Capsule())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.secondary)
        .accessibilityLabel("Task: \(task.title), \(task.state.label)")
        .accessibilityHint("Opens the task")
    }
}
