import SwiftUI

struct TaskListView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @State private var filter: TaskFilter = .all
    @State private var search = ""
    @State private var priority: TaskPriority?
    @State private var project: String?
    @State private var agentId: String?

    private var filtered: [HBTask] {
        store.tasks.filter { t in
            filter.matches(t.state)
                && (search.isEmpty || t.title.localizedCaseInsensitiveContains(search))
                && (priority == nil || t.priorityLevel == priority)
                && (project == nil || t.project == project)
                && (agentId == nil || t.agentId == agentId)
        }
        .sorted(by: Self.order)
    }

    /// Active work first, then by priority, then newest.
    static func order(_ a: HBTask, _ b: HBTask) -> Bool {
        func group(_ t: HBTask) -> Int {
            switch t.state {
            case .failed, .blocked, .waitingApproval: 0
            case .inProgress: 1
            case .ready, .inbox, .unknown: 2
            case .done, .cancelled: 3
            }
        }
        if group(a) != group(b) { return group(a) < group(b) }
        if a.priorityLevel.rank != b.priorityLevel.rank { return a.priorityLevel.rank < b.priorityLevel.rank }
        return a.updatedAt > b.updatedAt
    }

    /// Top-level tasks with their subtasks nested, unless a filter makes nesting misleading.
    private var showsTree: Bool { filter == .all && search.isEmpty && priority == nil && project == nil && agentId == nil }

    private var hasFilters: Bool { priority != nil || project != nil || agentId != nil }

    var body: some View {
        Group {
            if store.info != nil && !store.supportsTasks {
                EmptyState(symbol: "checklist", title: "Tasks need a newer Loft",
                           message: "This server doesn't have the task API yet. Pull the latest Loft on your Mac and restart it.")
            } else {
                list
            }
        }
        .navigationTitle("Tasks")
        .createMenu()
    }

    private var list: some View {
        List {
            StaleBanner().listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
            Section {
                filterBar.listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 4, trailing: 0)).listRowBackground(Color.clear)
            }
            let items = filtered
            if showsTree {
                let ids = Set(items.map(\.id))
                ForEach(items.filter { $0.parentId == nil || !ids.contains($0.parentId!) }) { t in
                    TaskTreeRows(task: t, depth: 0)
                }
            } else {
                ForEach(items) { t in
                    NavigationLink(value: TaskRoute(id: t.id)) { TaskRow(task: t, showParent: true) }
                }
            }
        }
        .overlay {
            if !store.hasLoaded {
                NotLoadedView()
            } else if filtered.isEmpty {
                if !search.isEmpty {
                    ContentUnavailableView.search(text: search)
                } else {
                    EmptyState(symbol: filter == .all ? "checklist" : "line.3.horizontal.decrease", title: hasFilters ? "No matching tasks" : "Nothing here",
                               message: hasFilters ? "Try clearing the filters." : filter.emptyMessage,
                               actionTitle: filter == .all || filter == .inbox ? "New task" : nil,
                               action: { router.composer = .newTask(agentId: nil, parentId: nil) })
                }
            }
        }
        .searchable(text: $search, prompt: "Search tasks")
        .refreshable { try? await store.refresh() }
        .toolbar {
            ToolbarItem(placement: .topBarLeading) { filterMenu }
        }
    }

    private var filterBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(TaskFilter.allCases) { f in
                    let count = store.tasks.filter { f.matches($0.state) }.count
                    Button {
                        withAnimation(.snappy) { filter = f }
                    } label: {
                        HStack(spacing: 4) {
                            Text(f.label)
                            if count > 0 && f != .all && f != .done { Text("\(count)").foregroundStyle(filter == f ? .white.opacity(0.8) : .secondary) }
                        }
                        .font(.subheadline.weight(.medium))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(filter == f ? Theme.accent : Theme.card, in: Capsule())
                        .foregroundStyle(filter == f ? .white : .primary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(filter == f ? .isSelected : [])
                    .accessibilityIdentifier("taskFilter-\(f.rawValue)")
                }
            }
            .padding(.horizontal, Theme.gutter)
        }
    }

    private var filterMenu: some View {
        Menu {
            Picker("Priority", selection: $priority) {
                Text("Any priority").tag(TaskPriority?.none)
                ForEach(TaskPriority.allCases) { Text($0.label).tag(TaskPriority?.some($0)) }
            }
            Picker("Project", selection: $project) {
                Text("Any project").tag(String?.none)
                ForEach(store.projects, id: \.self) { Text($0).tag(String?.some($0)) }
            }
            Picker("Agent", selection: $agentId) {
                Text("Anyone").tag(String?.none)
                ForEach(store.agents) { Text($0.name).tag(String?.some($0.id)) }
            }
            if hasFilters {
                Button("Clear filters", role: .destructive) { priority = nil; project = nil; agentId = nil }
            }
        } label: {
            Image(systemName: hasFilters ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .accessibilityLabel("Filter tasks")
    }
}

/// A task and, indented beneath it, its subtasks.
private struct TaskTreeRows: View {
    @Environment(LoftStore.self) private var store
    let task: HBTask
    let depth: Int

    var body: some View {
        NavigationLink(value: TaskRoute(id: task.id)) {
            TaskRow(task: task)
                .padding(.leading, CGFloat(depth) * 18)
        }
        if depth < 4 {
            ForEach(store.subtasks(of: task.id).filter { $0.state != .cancelled }) { child in
                TaskTreeRows(task: child, depth: depth + 1)
            }
        }
    }
}

struct TaskRow: View {
    @Environment(LoftStore.self) private var store
    let task: HBTask
    var showParent = false

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: task.state.symbol)
                .foregroundStyle(task.state.color)
                .font(.body)
                .frame(width: 22)
                .padding(.top, 1)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 4) {
                    if let sym = task.priorityLevel.symbol {
                        Image(systemName: sym).font(.caption.weight(.bold)).foregroundStyle(Theme.failure)
                            .accessibilityLabel("\(task.priorityLevel.label) priority")
                    }
                    Text(task.title)
                        .font(.body)
                        .strikethrough(task.state == .done || task.state == .cancelled, color: .secondary)
                        .foregroundStyle(task.state.isClosed ? .secondary : .primary)
                        .readableLineLimit(2)
                }
                if showParent, let parent = store.task(task.parentId) {
                    Text("↳ \(parent.title)").font(.caption).foregroundStyle(.secondary).readableLineLimit(1)
                }
                meta
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityValue(task.state.label)
        .accessibilityIdentifier("taskRow-\(task.title)")
    }

    private var meta: some View {
        let children = store.subtasks(of: task.id)
        let parts: [String] = [
            task.state == .inbox || task.state == .ready ? nil : task.state.label,
            store.agent(task.agentId)?.name,
            task.isPersonalTodo && task.agentId == nil ? "Personal" : nil,
            children.isEmpty ? nil : "\(children.filter { $0.state == .done }.count)/\(children.count) subtasks",
            task.dueAt.map { "Due \($0.formatted(.dateTime.month(.abbreviated).day()))" },
            task.project.isEmpty ? nil : task.project,
        ].compactMap { $0 }
        return VStack(alignment: .leading, spacing: 2) {
            if !parts.isEmpty {
                Text(parts.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary).readableLineLimit(1)
            }
            if task.state.isActive, !task.note.isEmpty {
                Text(task.note).font(.caption).foregroundStyle(.secondary).readableLineLimit(1)
            }
        }
    }
}
