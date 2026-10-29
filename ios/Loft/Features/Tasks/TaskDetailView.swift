import SwiftUI

struct TaskDetailView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @Environment(\.dismiss) private var dismiss
    let taskId: String
    @State private var error: APIError?
    @State private var busy = false
    @State private var confirmCancel = false
    @State private var confirmDelete = false
    @State private var confirmPublish = false
    @State private var pickAgent = false
    @State private var notice: String?
    /// Opened from a link or a push before the app has this task (a cold start, or a new task): look it up first.
    @State private var lookedUp = false

    var body: some View {
        if let task = store.task(taskId) {
            content(task)
        } else if !lookedUp {
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .task {
                    // Up to ten seconds while the app connects; one successful refresh settles it.
                    for _ in 0..<20 {
                        if store.task(taskId) != nil { break }
                        if store.api != nil, (try? await store.refresh()) != nil { break }
                        try? await Task.sleep(for: .milliseconds(500))
                    }
                    lookedUp = true
                }
        } else {
            EmptyState(symbol: "checklist", title: "Task not found", message: "It may have been deleted.")
        }
    }

    private func content(_ t: HBTask) -> some View {
        let children = store.subtasks(of: t.id)
        let agent = store.agent(t.agentId)
        return List {
            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Text(t.title).font(.title3.weight(.semibold))
                    HStack {
                        StatusBadge(t.state)
                        if t.priorityLevel != .normal {
                            StatusBadge(text: t.priorityLevel.label, symbol: t.priorityLevel.symbol ?? "arrow.down",
                                        color: t.priorityLevel.rank <= 1 ? Theme.failure : .secondary)
                        }
                        if t.isPersonalTodo && t.agentId == nil {
                            StatusBadge(text: "Personal", symbol: "person", color: .secondary)
                        }
                    }
                    if let labels = t.labels, !labels.isEmpty {
                        IssueLabels(labels: labels)
                    }
                }
                .padding(.vertical, 4)
                if !t.description.isEmpty {
                    CollapsibleText(text: t.description, lineLimit: 8).font(.subheadline)
                }
                if let notice {
                    Label(notice, systemImage: "checkmark.circle").font(.footnote).foregroundStyle(Theme.working)
                }
            }

            // Where it lives. GitHub owns the title, description, labels, owner and open/closed of a mirrored issue.
            Section("Where") {
                if let g = t.github {
                    // Only a web link: a malformed stored value can't open another app's scheme.
                    if let url = URL(string: g.url), url.scheme == "https" {
                        Link(destination: url) {
                            LabeledContent("On GitHub") { IssueOriginMark(link: g) }
                        }
                        .accessibilityIdentifier("issueGitHubLink")
                    }
                    LabeledContent("On GitHub it's", value: g.state == "closed" ? "Closed" : "Open")
                    if let sub = g.subIssues { LabeledContent("Sub-issues", value: sub) }
                } else {
                    LabeledContent("Only in Loft", value: t.project.isEmpty ? "No project" : t.project)
                    // Its project's repo gets it as a GitHub issue; the server says why when it can't.
                    if !t.project.isEmpty, !t.state.isClosed {
                        Button { confirmPublish = true } label: { Label("Publish to GitHub", systemImage: "arrow.up.forward.square") }
                        .disabled(!store.canWrite || busy)
                        .accessibilityIdentifier("publishTask")
                        // As on the web: say where it goes before a possibly public issue is created.
                        .confirmationDialog("Publish to GitHub?", isPresented: $confirmPublish, titleVisibility: .visible) {
                            Button("Publish") {
                                act {
                                    let p = try await store.publishTask(t.id)
                                    notice = p.github.map { "Published as \($0.shortName)." }
                                }
                            }
                            .accessibilityIdentifier("confirmPublish")
                        } message: {
                            Text("It becomes an issue in the \(t.project) project's GitHub repo, which may be public.")
                        }
                    }
                }
                if let o = t.ownerLogin { LabeledContent("Owner", value: "@\(o)") }
            }

            if t.state.isActive && !t.note.isEmpty {
                Section("Latest progress") {
                    Text(t.note).font(.subheadline)
                }
            }
            if !t.summary.isEmpty {
                Section("Summary") { CollapsibleText(text: t.summary).font(.subheadline) }
            }
            if !t.error.isEmpty {
                Section("Problem") {
                    Label(t.error, systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline)
                        .foregroundStyle(t.state == .failed ? Theme.failure : Theme.attention)
                }
            }

            Section { actions(t) }

            Section("Assignment") {
                if let agent {
                    NavigationLink(value: AgentRoute(id: agent.id)) {
                        HStack(spacing: 10) {
                            AgentAvatar(name: agent.name, id: agent.id, size: 28, state: store.state(of: agent))
                            VStack(alignment: .leading) {
                                Text(agent.name)
                                Text(store.state(of: agent).label).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                    Button { router.openChat(agent.id) } label: { Label("Open conversation", systemImage: "bubble.left.and.bubble.right") }
                } else {
                    Text("Not assigned").foregroundStyle(.secondary)
                }
                if t.isDelegated {
                    LabeledContent("Delegated by", value: store.agent(t.createdBy)?.name ?? "An agent")
                }
            }

            if let parent = store.task(t.parentId) {
                Section("Part of") {
                    NavigationLink(value: TaskRoute(id: parent.id)) { TaskRow(task: parent) }
                }
            }

            Section {
                if children.isEmpty {
                    Text("No subtasks.").foregroundStyle(.secondary)
                } else {
                    ForEach(children) { c in
                        NavigationLink(value: TaskRoute(id: c.id)) { TaskRow(task: c) }
                    }
                }
                Button { router.composer = .newTask(agentId: nil, parentId: t.id) } label: { Label("Add subtask", systemImage: "plus") }
                    .disabled(!store.canWrite)
            } header: {
                Text(children.isEmpty ? "Subtasks" : "Subtasks · \(children.filter { $0.state == .done }.count)/\(children.count) done")
            }

            if !t.dependsOn.isEmpty {
                Section("Waits for") {
                    ForEach(t.dependsOn, id: \.self) { id in
                        if let d = store.task(id) {
                            NavigationLink(value: TaskRoute(id: d.id)) { TaskRow(task: d) }
                        }
                    }
                }
            }

            TaskThreadSection(task: t)

            let history = store.activity.filter { $0.taskId == t.id }
            if !history.isEmpty {
                Section("History") {
                    ForEach(history) { ActivityRow(item: $0) }
                }
            }

            Section("Details") {
                if !t.project.isEmpty { LabeledContent("Project", value: t.project) }
                if let due = t.dueAt { LabeledContent("Due", value: due.formatted(date: .abbreviated, time: .shortened)) }
                LabeledContent("Created", value: t.createdAt.formatted(date: .abbreviated, time: .shortened))
                LabeledContent("Updated", value: t.updatedAt.formatted(date: .abbreviated, time: .shortened))
                if let s = t.startedAt { LabeledContent("Started", value: s.formatted(date: .abbreviated, time: .shortened)) }
                if let c = t.completedAt { LabeledContent("Finished", value: c.formatted(date: .abbreviated, time: .shortened)) }
            }
        }
        .navigationTitle("Task")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button { router.composer = .editTask(t) } label: { Label("Edit", systemImage: "pencil") }
                    if !t.state.isActive {
                        Button(role: .destructive) { confirmDelete = true } label: { Label("Delete", systemImage: "trash") }
                    }
                } label: { Image(systemName: "ellipsis.circle") }
                .disabled(!store.canWrite)
                .accessibilityLabel("Task options")
            }
        }
        .confirmationDialog("Cancel this task?", isPresented: $confirmCancel, titleVisibility: .visible) {
            Button("Cancel task", role: .destructive) { act { try await store.cancelTask(t.id, stopRun: false) } }
            if let agent, agent.running == true {
                Button("Cancel task and stop \(agent.name)'s run", role: .destructive) { act { try await store.cancelTask(t.id, stopRun: true) } }
            }
            Button("Keep working", role: .cancel) {}
        } message: {
            Text(agent.map { "\($0.name) is told to drop it. Stopping the run also ends anything else it is doing right now." } ?? "")
        }
        .confirmationDialog("Delete this task?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                act {
                    try await store.deleteTask(t.id)
                    dismiss()
                }
            }
        } message: {
            Text(children.isEmpty ? "This can't be undone." : "Its subtasks are kept as top-level tasks.")
        }
        .sheet(isPresented: $pickAgent) {
            NavigationStack {
                AgentPickerList(title: "Run with…") { a in
                    pickAgent = false
                    run(t, agentId: a.id)
                }
            }
            .presentationDetents([.medium, .large])
        }
        .errorAlert($error)
        .refreshable { try? await store.refresh() }
    }

    @ViewBuilder private func actions(_ t: HBTask) -> some View {
        let writable = store.canWrite && !busy
        if t.state.canExecute {
            if let a = store.agent(t.agentId) {
                Button { run(t, agentId: a.id) } label: {
                    Label(t.state == .failed ? "Retry with \(a.name)" : "Run with \(a.name)", systemImage: t.state == .failed ? "arrow.clockwise" : "play.fill")
                }
                .disabled(!writable)
                .accessibilityIdentifier("runTask")
            }
            Button { pickAgent = true } label: { Label(t.agentId == nil ? "Run with an agent…" : "Run with another agent…", systemImage: "person.badge.plus") }
                .disabled(!writable)
            if let o = store.orchestrator, o.id != t.agentId {
                Button {
                    act {
                        let r = try await store.delegateTask(t.id)
                        notice = r.queued == true ? "\(o.name) will plan it after its current run." : "\(o.name) is planning it."
                    }
                } label: { Label("Ask \(o.name) to plan & delegate", systemImage: "sparkles") }
                .disabled(!writable)
                .accessibilityIdentifier("delegateTask")
            }
        }
        if t.state.isActive {
            Button(role: .destructive) { confirmCancel = true } label: { Label("Cancel task", systemImage: "xmark.circle").foregroundStyle(Theme.failure) }
                .disabled(!writable)
        }
        if !t.state.isActive && !t.state.isClosed {
            Button { act { try await store.updateTask(t.id, ["status": "done"]) } } label: {
                Label("Mark as done", systemImage: "checkmark.circle")
            }
            .disabled(!writable)
        }
        if t.state.isClosed {
            Button { act { try await store.updateTask(t.id, ["status": "inbox"]) } } label: {
                Label("Reopen", systemImage: "arrow.uturn.backward.circle")
            }
            .disabled(!writable)
        }
        if !store.canWrite {
            Text("Reconnect to change this task.").font(.footnote).foregroundStyle(.secondary)
        }
    }

    private func run(_ t: HBTask, agentId: String) {
        act {
            let r = try await store.executeTask(t.id, agentId: agentId)
            let name = store.agent(agentId)?.name ?? "The agent"
            notice = r.queued == true ? "Sent. \(name) starts after its current run." : "Sent. \(name) is on it."
        }
    }

    private func act(_ f: @escaping () async throws -> Void) {
        busy = true
        notice = nil
        Task {
            do { try await f() } catch { self.error = APIError.from(error) }
            busy = false
        }
    }
}

struct AgentPickerList: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let title: String
    let pick: (Agent) -> Void

    var body: some View {
        List(store.sortedAgents) { a in
            Button { pick(a) } label: { AgentRow(agent: a) }
                .buttonStyle(.plain)
        }
        .contentMargins(.top, 8, for: .scrollContent)
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
    }
}
