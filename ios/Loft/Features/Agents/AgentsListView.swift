import SwiftUI

struct AgentsListView: View {
    @Environment(LoftStore.self) private var store
    @State private var search = ""
    @State private var stateFilter: AgentState?
    @State private var project: String?

    private var filtered: [Agent] {
        store.sortedAgents.filter { a in
            (search.isEmpty || a.name.localizedCaseInsensitiveContains(search))
                && (stateFilter == nil || store.state(of: a) == stateFilter)
                && (project == nil || a.project == project)
        }
    }

    private var grouped: [(String, [Agent])] {
        Dictionary(grouping: filtered) { $0.project?.isEmpty == false ? $0.project! : "No project" }
            .sorted { $0.key.localizedCaseInsensitiveCompare($1.key) == .orderedAscending }
    }

    var body: some View {
        List {
            StaleBanner().listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
            if search.isEmpty { Section { DigestCard() } }
            ForEach(grouped, id: \.0) { project, agents in
                Section(project) {
                    ForEach(agents) { a in
                        NavigationLink(value: AgentRoute(id: a.id)) { AgentRow(agent: a) }
                    }
                }
            }
        }
        .overlay {
            if !store.hasLoaded {
                NotLoadedView()
            } else if store.agents.isEmpty {
                EmptyState(symbol: "person.3", title: "No agents registered",
                           message: "Agents are created in Loft on your Mac. They show up here once they exist.")
            } else if filtered.isEmpty {
                ContentUnavailableView.search(text: search)
            }
        }
        .searchable(text: $search, prompt: "Search agents")
        .refreshable { try? await store.refresh() }
        .navigationTitle("Agents")
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Menu {
                    Picker("Status", selection: $stateFilter) {
                        Text("Any status").tag(AgentState?.none)
                        ForEach([AgentState.working, .waitingApproval, .idle, .failed], id: \.self) { s in
                            Label(s.label, systemImage: s.symbol).tag(AgentState?.some(s))
                        }
                    }
                    Picker("Project", selection: $project) {
                        Text("Any project").tag(String?.none)
                        ForEach(Array(Set(store.agents.compactMap(\.project).filter { !$0.isEmpty })).sorted(), id: \.self) {
                            Text($0).tag(String?.some($0))
                        }
                    }
                } label: {
                    Image(systemName: stateFilter == nil && project == nil
                          ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
                }
                .accessibilityLabel("Filter agents")
            }
        }
        .createMenu()
    }
}

struct AgentRow: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dynamicTypeSize) private var typeSize
    let agent: Agent

    var body: some View {
        let state = store.state(of: agent)
        HStack(spacing: 12) {
            // At accessibility sizes the avatar's width is what made names hyphenate; the state is in the chip.
            if !typeSize.isAccessibilitySize {
                AgentAvatar(name: agent.name, id: agent.id, size: 38, state: state)
            }
            VStack(alignment: .leading, spacing: 3) {
                if typeSize.isAccessibilitySize {
                    AdaptiveStack(spacing: 4) {
                        Text(agent.name).font(.body.weight(.semibold)).readableLineLimit(2)
                        ChipRow { chips(state) }
                    }
                } else {
                    // Chips beside the name when everything fits on one line; otherwise together on the
                    // line below, never stacked on top of each other, so rows look alike.
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 4) {
                            Text(agent.name).font(.body.weight(.semibold)).lineLimit(1)
                            Spacer(minLength: 4)
                            HStack(spacing: 6) { chips(state) }
                        }
                        VStack(alignment: .leading, spacing: 3) {
                            Text(agent.name).font(.body.weight(.semibold)).lineLimit(2)
                            HStack(spacing: 6) { chips(state) }
                        }
                    }
                }
                if let t = store.currentTask(for: agent.id) {
                    Label(t.title, systemImage: "checklist")
                        .font(.footnote)
                        .readableLineLimit(1)
                }
                Text(subtitle(state))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .readableLineLimit(1)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("agentRow-\(agent.name)")
    }

    @ViewBuilder private func chips(_ state: AgentState) -> some View {
        if let ctx = agent.contextTokens { ContextChip(tokens: ctx) }
        StatusBadge(state, compact: true)
    }

    private func subtitle(_ s: AgentState) -> String {
        if let t = agent.statusText, !t.isEmpty { return t }
        if s == .waitingApproval, let p = store.pendingApprovals(for: agent.id).first { return p.summary }
        if let last = agent.lastRunAt { return "Last ran \(last.shortAgo == "now" ? "just now" : last.shortAgo + " ago")" }
        return agent.title?.isEmpty == false ? agent.title! : "Hasn't run yet"
    }
}
