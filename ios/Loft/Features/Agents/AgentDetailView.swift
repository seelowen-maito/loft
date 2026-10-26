import SwiftUI

struct AgentDetailView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    let agentId: String
    @State private var confirmStop = false
    @State private var error: APIError?
    @State private var busy = false
    @State private var showAllActivity = false

    var body: some View {
        if let agent = store.agent(agentId) {
            content(agent)
        } else {
            EmptyState(symbol: "person.crop.circle.badge.questionmark", title: "Agent not found",
                       message: "It may have been deleted on the Mac.")
        }
    }

    private func content(_ agent: Agent) -> some View {
        let state = store.state(of: agent)
        let current = store.currentTask(for: agent.id)
        let assigned = store.tasks.filter { $0.agentId == agent.id && !$0.state.isClosed && $0.id != current?.id }
        let activity = store.activity.filter { $0.agentId == agent.id }
        return List {
            Section {
                HStack(spacing: 14) {
                    AgentAvatar(name: agent.name, id: agent.id, size: 56, state: state)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(agent.name).font(.title3.weight(.semibold))
                        if let t = agent.title, !t.isEmpty { Text(t).font(.subheadline).foregroundStyle(.secondary) }
                        StatusBadge(state)
                    }
                }
                .padding(.vertical, 4)
                if let text = agent.statusText, !text.isEmpty {
                    Text(text).font(.subheadline)
                }
                if state == .working, let started = agent.lastRunAt {
                    TimelineView(.periodic(from: .now, by: 30)) { ctx in
                        LabeledContent("Running for", value: started.durationSince(ctx.date))
                    }
                }
            }

            Section {
                actions(agent, state: state)
            }

            let approvals = store.pendingApprovals(for: agent.id)
            if !approvals.isEmpty {
                Section("Waiting for you") {
                    ForEach(approvals) { ApprovalCard(request: $0).listRowInsets(EdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0)) }
                }
                .listRowBackground(Color.clear)
            }

            if store.supportsTasks {
                Section("Tasks") {
                    if let current {
                        NavigationLink(value: TaskRoute(id: current.id)) { TaskRow(task: current) }
                    } else {
                        Text(state == .working ? "Working outside a tracked task." : "No task assigned right now.")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(assigned) { t in
                        NavigationLink(value: TaskRoute(id: t.id)) { TaskRow(task: t) }
                    }
                }
            }

            DelegationSection(agent: agent)

            if let summary = store.summaries[agent.id] {
                AgentStatsSection(summary: summary, contextTokens: agent.contextTokens)
            }

            Section("Recent activity") {
                if activity.isEmpty {
                    Text("Nothing recent.").foregroundStyle(.secondary)
                } else {
                    ForEach(activity.prefix(showAllActivity ? 50 : 6)) { ActivityRow(item: $0) }
                    if activity.count > 6 && !showAllActivity {
                        Button("Show more") { showAllActivity = true }
                    }
                }
            }

            let prs = store.pullRequests.filter { $0.agentId == agent.id }.sorted { ($0.createdAt ?? .distantPast) > ($1.createdAt ?? .distantPast) }
            if !prs.isEmpty {
                Section("Pull requests") {
                    ForEach(prs.prefix(5)) { pr in
                        if let url = URL(string: pr.url) {
                            Link(destination: url) {
                                HStack {
                                    Image(systemName: pr.state == "merged" ? "arrow.triangle.merge" : pr.state == "open" ? "arrow.triangle.pull" : "xmark.circle")
                                        .foregroundStyle(pr.state == "merged" ? Theme.accent : pr.state == "open" ? Theme.working : .secondary)
                                    VStack(alignment: .leading) {
                                        Text(pr.title).lineLimit(2)
                                        Text("\(pr.slug) #\(pr.number) · \(pr.state)").font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Section("Details") {
                LabeledContent("Agent ID") {
                    Text(agent.id).font(.caption.monospaced()).textSelection(.enabled).lineLimit(1).truncationMode(.middle)
                }
                if let p = agent.project, !p.isEmpty { LabeledContent("Project", value: p) }
                if let m = agent.model, !m.isEmpty { LabeledContent("Model", value: [m, agent.effort ?? ""].filter { !$0.isEmpty }.joined(separator: " · ")) }
                if let mode = agent.permissionMode { LabeledContent("Permissions", value: Self.describe(mode: mode)) }
                LabeledContent("Schedule", value: (agent.intervalMinutes ?? 0) > 0 ? "Every \(Self.describe(minutes: agent.intervalMinutes!))" : "Only when messaged")
                if let next = agent.nextRunAt, (agent.intervalMinutes ?? 0) > 0 { LabeledContent("Next wake-up", value: next.formatted(date: .omitted, time: .shortened)) }
                if let last = agent.lastRunAt { LabeledContent("Last run", value: last.formatted(date: .abbreviated, time: .shortened)) }
            }
        }
        .navigationTitle(agent.name)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable {
            async let stats: Void = store.refreshStats()
            try? await store.refresh()
            await stats
        }
        .confirmationDialog("Stop \(agent.name)'s current run?", isPresented: $confirmStop, titleVisibility: .visible) {
            Button("Stop run", role: .destructive) { act { try await store.stopRun(agent.id) } }
        } message: {
            Text("This ends the Claude process running now. Messages waiting for it are dropped and pending approvals are denied. Its tasks stay as they are; cancel them separately.")
        }
        .errorAlert($error)
    }

    @ViewBuilder private func actions(_ agent: Agent, state: AgentState) -> some View {
        Button { router.openChat(agent.id) } label: { Label("Chat with \(agent.name)", systemImage: "bubble.left.and.bubble.right") }
            .accessibilityIdentifier("chatWithAgent")
        if store.supportsTasks {
            Button { router.composer = .newTask(agentId: agent.id, parentId: nil) } label: { Label("Assign a task", systemImage: "checklist") }
                .disabled(!store.canWrite)
        }
        Button { router.openChat(agent.id, draft: "What's your status? Summarize what you're working on and anything blocking you.") } label: {
            Label("Ask for a status update", systemImage: "text.bubble")
        }
        if state == .working || state == .waitingApproval || agent.running == true {
            Button(role: .destructive) { confirmStop = true } label: { Label("Stop current run", systemImage: "stop.circle").foregroundStyle(Theme.failure) }
                .disabled(!store.canWrite || busy)
        } else {
            Button { act { try await store.runNow(agent.id) } } label: { Label("Wake up now", systemImage: "play.circle") }
                .disabled(!store.canWrite || busy)
        }
    }

    private func act(_ f: @escaping () async throws -> Void) {
        busy = true
        Task {
            do { try await f() } catch { self.error = APIError.from(error) }
            busy = false
        }
    }

    static func describe(mode: String) -> String {
        switch mode {
        case "auto": "Auto (risky actions ask)"
        case "plan": "Plan first"
        case "acceptEdits": "Accept edits"
        case "bypassPermissions": "Bypass all"
        case "manual", "default": "Ask for everything"
        case "dontAsk": "Don't ask (deny)"
        default: mode
        }
    }

    static func describe(minutes: Int) -> String {
        minutes % 60 == 0 ? "\(minutes / 60)h" : "\(minutes) min"
    }
}

/// Who this agent works with, from real data: tasks it delegated or was handed
/// by another agent, and agents that messaged it.
/// One agent's last two weeks: runs per day, counts, top tools, PRs and memories.
private struct AgentStatsSection: View {
    let summary: AgentSummary
    let contextTokens: Int?

    var body: some View {
        Section {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline) {
                    Text("\(summary.runs) runs").font(.headline).monospacedDigit()
                    Text("\(summary.dailyAuto.reduce(0, +)) scheduled · \(summary.dailyUser.reduce(0, +)) requested")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Sparkline(days: ActivityDay.build(auto: summary.dailyAuto, user: summary.dailyUser))
            }
            .padding(.vertical, 4)
            .accessibilityElement(children: .combine)

            LabeledContent("Tool calls", value: "\(summary.toolCalls)")
            LabeledContent("Messages from you", value: "\(summary.userMessages)")
            LabeledContent("Memories", value: "\(summary.memories)")
            LabeledContent("Pull requests") {
                Text("\(summary.prs.merged) merged · \(summary.prs.open) open").monospacedDigit()
            }
            if let ctx = contextTokens {
                LabeledContent("Context") { ContextChip(tokens: ctx) }
            }
            let tools = summary.topToolCounts.prefix(5)
            if !tools.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Top tools").font(.subheadline.weight(.semibold))
                    let most = max(1, tools.map(\.count).max() ?? 1)
                    ForEach(Array(tools), id: \.name) { t in
                        HStack(spacing: 8) {
                            Text(t.name).font(.footnote.monospaced()).lineLimit(1).frame(width: 110, alignment: .leading)
                            GeometryReader { g in
                                Capsule().fill(Theme.scheduled.opacity(0.85))
                                    .frame(width: max(4, g.size.width * CGFloat(t.count) / CGFloat(most)))
                            }
                            .frame(height: 6)
                            Text("\(t.count)").font(.footnote.monospacedDigit()).foregroundStyle(.secondary)
                                .frame(minWidth: 32, alignment: .trailing)
                        }
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("\(t.name), \(t.count) calls")
                    }
                }
                .padding(.vertical, 4)
            }
        } header: {
            Text("Last \(summary.days) days")
        }
    }
}

private struct DelegationSection: View {
    @Environment(LoftStore.self) private var store
    let agent: Agent

    var body: some View {
        let delegatedTo = Set(store.tasks.filter { $0.createdBy == agent.id }.compactMap(\.agentId)).subtracting([agent.id])
        let delegatedBy = Set(store.tasks.filter { $0.agentId == agent.id && $0.isDelegated }.map(\.createdBy)).subtracting([agent.id])
        let messagedBy = Set(store.conversation(agent.id).compactMap(\.fromAgentId)).subtracting(delegatedBy)
        let isOrchestrator = store.orchestratorId == agent.id
        if !delegatedTo.isEmpty || !delegatedBy.isEmpty || !messagedBy.isEmpty || isOrchestrator {
            Section {
                if isOrchestrator {
                    Label("Your orchestrator", systemImage: "sparkles").foregroundStyle(Theme.accent)
                }
                ForEach(Array(delegatedBy), id: \.self) { link($0, "Hands work to \(agent.name)") }
                ForEach(Array(delegatedTo), id: \.self) { link($0, "Gets work from \(agent.name)") }
                ForEach(Array(messagedBy), id: \.self) { link($0, "Messaged \(agent.name)") }
            } header: {
                Text("Works with")
            } footer: {
                Text("From delegated tasks and agent-to-agent messages.")
            }
        }
    }

    @ViewBuilder private func link(_ id: String, _ caption: String) -> some View {
        if let a = store.agent(id) {
            NavigationLink(value: AgentRoute(id: a.id)) {
                HStack(spacing: 10) {
                    AgentAvatar(name: a.name, id: a.id, size: 28, state: store.state(of: a))
                    VStack(alignment: .leading) {
                        Text(a.name)
                        Text(caption).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }
}
