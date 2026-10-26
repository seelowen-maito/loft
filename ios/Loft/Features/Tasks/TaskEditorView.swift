import SwiftUI

/// Create or edit a task. Only fields the user owns are editable; execution status
/// belongs to the agent and changes through Run / Cancel.
struct TaskEditorView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @Environment(\.dismiss) private var dismiss

    private let editing: HBTask?
    /// Sent as the task id on create, so a retried save can't make a duplicate.
    @State private var draftId = UUID().uuidString
    @State private var title = ""
    @State private var details = ""
    @State private var priority: TaskPriority = .normal
    @State private var project = ""
    @State private var agentId: String?
    @State private var parentId: String?
    @State private var hasDue = false
    @State private var due = Calendar.current.date(byAdding: .day, value: 1, to: .now) ?? .now
    @State private var saving = false
    @State private var error: APIError?
    @FocusState private var titleFocused: Bool

    init(agentId: String? = nil, parentId: String? = nil) {
        editing = nil
        _agentId = State(initialValue: agentId)
        _parentId = State(initialValue: parentId)
    }

    init(editing t: HBTask) {
        editing = t
        _title = State(initialValue: t.title)
        _details = State(initialValue: t.description)
        _priority = State(initialValue: t.priorityLevel)
        _project = State(initialValue: t.project)
        _agentId = State(initialValue: t.agentId)
        _parentId = State(initialValue: t.parentId)
        _hasDue = State(initialValue: t.dueAt != nil)
        _due = State(initialValue: t.dueAt ?? Calendar.current.date(byAdding: .day, value: 1, to: .now) ?? .now)
    }

    private var valid: Bool { !title.trimmingCharacters(in: .whitespaces).isEmpty }
    private var assignmentLocked: Bool { editing?.state.isActive ?? false }

    var body: some View {
        Form {
            Section {
                TextField("Title", text: $title, axis: .vertical)
                    .font(.headline)
                    .focused($titleFocused)
                    .accessibilityIdentifier("taskTitleField")
                TextField("Description", text: $details, axis: .vertical)
                    .lineLimit(3...10)
            }
            Section {
                Picker("Priority", selection: $priority) {
                    ForEach(TaskPriority.allCases) { Text($0.label).tag($0) }
                }
                HStack {
                    TextField("Project", text: $project)
                    if !store.projects.isEmpty {
                        Menu {
                            ForEach(store.projects, id: \.self) { p in Button(p) { project = p } }
                        } label: { Image(systemName: "chevron.up.chevron.down").font(.footnote.weight(.semibold)) }
                        .tint(.secondary)
                        .accessibilityLabel("Choose a project")
                    }
                }
                Toggle("Due date", isOn: $hasDue.animation())
                if hasDue {
                    DatePicker("Due", selection: $due)
                }
            }
            Section {
                Picker("Agent", selection: $agentId) {
                    Text("Nobody (personal todo)").tag(String?.none)
                    ForEach(store.sortedAgents) { Text($0.name).tag(String?.some($0.id)) }
                }
                .disabled(assignmentLocked)
                Picker("Part of", selection: $parentId) {
                    Text("None").tag(String?.none)
                    ForEach(store.tasks.filter { $0.id != editing?.id && !$0.state.isClosed }) { Text($0.title).lineLimit(1).tag(String?.some($0.id)) }
                }
            } footer: {
                if assignmentLocked {
                    Text("An agent is working on this task. Cancel it to reassign. Editing the description doesn't change what the agent was already told.")
                } else if editing == nil {
                    Text(agentId == nil
                         ? "Saved to your inbox. Nothing runs until you choose to run it."
                         : "Assigned but not started. Use Save & run to hand it over now.")
                }
            }
        }
        .navigationTitle(editing != nil ? "Edit task" : parentId != nil ? "New subtask" : "New task")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { save(run: false) }
                    .disabled(!valid || saving || !store.canWrite)
                    .accessibilityIdentifier("saveTask")
            }
            if editing == nil {
                ToolbarItemGroup(placement: .bottomBar) {
                    if let agent = store.agent(agentId) {
                        Button { save(run: true) } label: { Label("Save & run with \(agent.name)", systemImage: "play.fill").labelStyle(.titleAndIcon) }
                            .buttonStyle(.borderedProminent)
                            .disabled(!valid || saving || !store.canWrite)
                    } else if let o = store.orchestrator {
                        Button { save(run: true, delegate: true) } label: { Label("Save & ask \(o.name) to plan", systemImage: "sparkles").labelStyle(.titleAndIcon) }
                            .buttonStyle(.bordered)
                            .disabled(!valid || saving || !store.canWrite)
                    }
                }
            }
        }
        .overlay {
            if saving { ProgressView().controlSize(.large) }
        }
        .safeAreaInset(edge: .top) {
            if !store.canWrite {
                Label("You're offline. Reconnect to save.", systemImage: "wifi.slash")
                    .font(.footnote)
                    .foregroundStyle(Theme.attention)
                    .padding(8)
            }
        }
        .interactiveDismissDisabled(saving)
        .onAppear { if editing == nil { titleFocused = true } }
        .errorAlert($error)
    }

    private func save(run: Bool, delegate: Bool = false) {
        var f: [String: JSONValue] = [
            "title": .string(title.trimmingCharacters(in: .whitespacesAndNewlines)),
            "description": .string(details.trimmingCharacters(in: .whitespacesAndNewlines)),
            "priority": .string(priority.rawValue),
            "project": .string(project.trimmingCharacters(in: .whitespaces)),
            "parent_id": .optional(parentId),
            "due_at": hasDue ? .string(JSONCoding.formatDate(due)) : .null,
        ]
        if !assignmentLocked { f["agent_id"] = .optional(agentId) }
        saving = true
        Task {
            defer { saving = false }
            do {
                if let editing {
                    try await store.updateTask(editing.id, f)
                } else {
                    f["id"] = .string(draftId)
                    f["kind"] = .string(agentId == nil && !delegate ? "todo" : "work")
                    let t = try await store.createTask(f)
                    if run {
                        if delegate { try await store.delegateTask(t.id) } else { try await store.executeTask(t.id, agentId: agentId) }
                    }
                }
                dismiss()
            } catch {
                self.error = APIError.from(error)
            }
        }
    }
}
