import LoftKit
import SwiftUI

struct TaskListView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @State private var filter: TaskFilter = .all
    @State private var search = ""
    @State private var priority: TaskPriority?
    @State private var project: String?
    @State private var group: String?
    @State private var agentId: String?
    @State private var showDropped = false
    /// List or Specs, kept on the device.
    @AppStorage("issuesView") private var mode = IssuesMode.list

    enum IssuesMode: String { case list, specs }

    /// The group and project filters, shared by both views.
    private func inScope(_ t: HBTask) -> Bool {
        (group == nil || SpecViews.inGroup(t, group!, agents: store.agents)) && (project == nil || t.project == project)
    }

    private var groups: [String] { Array(Set(store.agents.map(\.team).filter { !$0.isEmpty })).sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending } }

    /// The projects to pick from: the group's agents' when a group is picked.
    private var projectChoices: [String] {
        guard let group else { return store.projects }
        return Array(Set(store.agents.filter { $0.team == group }.flatMap(\.allProjects))).sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    }

    private var modePicker: some View {
        Picker("View", selection: $mode) {
            Text("List").tag(IssuesMode.list)
            Text("Specs").tag(IssuesMode.specs)
        }
        .pickerStyle(.segmented)
        .accessibilityIdentifier("issuesViewSwitch")
        .listRowBackground(Color.clear)
    }

    private var filtered: [HBTask] {
        store.tasks.filter { t in
            filter.matches(t.state)
                && (search.isEmpty || t.title.localizedCaseInsensitiveContains(search))
                && (priority == nil || t.priorityLevel == priority)
                && inScope(t)
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
    private var showsTree: Bool { filter == .all && search.isEmpty && priority == nil && project == nil && group == nil && agentId == nil }

    /// The filters that apply in this view: Specs has no Priority or Agent filter.
    private var hasFilters: Bool { project != nil || group != nil || (mode == .list && (priority != nil || agentId != nil)) }

    private func clearFilters() { priority = nil; project = nil; group = nil; agentId = nil }

    var body: some View {
        Group {
            if store.info != nil && !store.supportsTasks {
                EmptyState(symbol: "checklist", title: "Issues need a newer Loft",
                           message: "This server doesn't have the issues API yet. Pull the latest Loft on your Mac and restart it.")
            } else {
                if mode == .specs {
                    SpecsView(specs: store.tasks.filter { SpecViews.isSpec($0) && inScope($0) }, showDropped: showDropped,
                              clearFilters: hasFilters ? clearFilters : nil) {
                        modePicker
                    }
                    .toolbar { ToolbarItem(placement: .topBarLeading) { filterMenu } }
                } else {
                    list
                }
            }
        }
        .navigationTitle("Issues")
        .createMenu()
    }

    private var list: some View {
        List {
            StaleBanner().listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
            modePicker
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
                    EmptyState(symbol: filter == .all ? "checklist" : "line.3.horizontal.decrease", title: hasFilters ? "No matching issues" : "Nothing here",
                               message: hasFilters ? "Try clearing the filters." : filter.emptyMessage,
                               actionTitle: filter == .all || filter == .inbox ? "New issue" : nil,
                               action: { router.composer = .newTask(agentId: nil, parentId: nil) })
                }
            }
        }
        .searchable(text: $search, prompt: "Search issues")
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
                        .foregroundStyle(filter == f ? Theme.onAccent : .primary)
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
            if mode == .list {
                Picker("Priority", selection: $priority) {
                    Text("Any priority").tag(TaskPriority?.none)
                    ForEach(TaskPriority.allCases) { Text($0.label).tag(TaskPriority?.some($0)) }
                }
            }
            if !groups.isEmpty {
                Picker("Group", selection: $group) {
                    Text("Any group").tag(String?.none)
                    ForEach(groups, id: \.self) { Text($0).tag(String?.some($0)) }
                }
            }
            Picker("Project", selection: $project) {
                Text("Any project").tag(String?.none)
                ForEach(projectChoices, id: \.self) { Text($0).tag(String?.some($0)) }
            }
            if mode == .list {
                Picker("Agent", selection: $agentId) {
                    Text("Anyone").tag(String?.none)
                    ForEach(store.agents) { Text($0.name).tag(String?.some($0.id)) }
                }
            }
            if mode == .specs {
                Toggle("Show dropped", isOn: $showDropped)
            }
            if hasFilters {
                Button("Clear filters", role: .destructive, action: clearFilters)
            }
        } label: {
            Image(systemName: hasFilters ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .accessibilityLabel("Filter issues")
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
            children.isEmpty ? nil : "\(children.filter { $0.state == .done }.count)/\(children.count) sub-issues",
            children.isEmpty ? task.github?.subIssues : nil,
            task.dueAt.map { "Due \($0.formatted(.dateTime.month(.abbreviated).day()))" },
            task.github == nil && !task.project.isEmpty ? task.project : nil,
        ].compactMap { $0 }
        return VStack(alignment: .leading, spacing: 2) {
            if !parts.isEmpty || task.github != nil || task.ownerLogin != nil {
                HStack(spacing: 4) {
                    // Where it lives: on GitHub (its repo and number), or only in Loft (the project, as before).
                    if let g = task.github {
                        IssueOriginMark(link: g)
                    }
                    if !parts.isEmpty {
                        Text(parts.joined(separator: " · ")).font(.caption).foregroundStyle(.secondary).readableLineLimit(1)
                    }
                    if let o = task.ownerLogin {
                        if !parts.isEmpty { Text("·").font(.caption).foregroundStyle(.secondary) }
                        OwnerBadge(login: o).font(.caption).foregroundStyle(.secondary).layoutPriority(1)
                    }
                }
            }
            if let labels = task.labels, !labels.isEmpty {
                IssueLabels(labels: labels, limit: 3)
            }
            if task.state.isActive, !task.note.isEmpty {
                Text(task.note).font(.caption).foregroundStyle(.secondary).readableLineLimit(1)
            }
        }
    }
}

/// A mirrored Loft issue's GitHub mark: "loft#42", struck through once it's closed on GitHub.
struct IssueOriginMark: View {
    let link: HBTask.GitHubLink

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: link.state == "closed" ? "checkmark.circle" : "arrow.triangle.pull")
            Text(link.shortName)
        }
        .font(.caption.weight(.semibold))
        .foregroundStyle(link.state == "closed" ? Color.secondary : Theme.accent)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("On GitHub, \(link.shortName)\(link.state == "closed" ? ", closed" : "")")
    }
}

/// GitHub-style labels as small chips; past `limit`, a "+2".
struct IssueLabels: View {
    let labels: [String]
    var limit = Int.max

    var body: some View {
        HStack(spacing: 4) {
            ForEach(labels.prefix(limit), id: \.self) { l in
                Text(l).font(.caption2.weight(.medium))
                    .padding(.horizontal, 6).padding(.vertical, 2)
                    .background(Color.secondary.opacity(0.15), in: Capsule())
            }
            if labels.count > limit {
                Text("+\(labels.count - limit)").font(.caption2).foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Labels: \(labels.joined(separator: ", "))")
    }
}
