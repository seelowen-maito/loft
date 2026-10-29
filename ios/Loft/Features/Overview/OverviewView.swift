import SwiftUI

struct OverviewView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @Environment(VoiceSession.self) private var voice
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// The Talk button steps aside while you scroll down through the cards, so it doesn't
    /// sit on top of rows you're reading; it comes back when you scroll up or reach either end.
    @State private var talkVisible = true

    private var working: [Agent] {
        store.sortedAgents.filter { [.working, .waitingApproval].contains(store.state(of: $0)) }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                ConnectionHeader()
                StaleBanner()
                VoiceDraftBanner()
                SummaryGrid()
                PlanUsageCard()
                if store.supportsNeedsYou { NeedsYouCard() } else { AttentionSection() }
                activeWork
                ActivityCard()
                recentActivity
            }
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, store.agents.isEmpty ? 24 : 96)
        }
        .background(Theme.background)
        .hidesOnScrollDown($talkVisible)
        .overlay(alignment: .bottomTrailing) {
            if !store.agents.isEmpty && talkVisible {
                TalkButton { agentId in Task { await voice.start(agentId: agentId) } }
                    .padding(.trailing, Theme.gutter)
                    .padding(.bottom, 16)
                    .transition(reduceMotion ? .opacity : .scale(scale: 0.6).combined(with: .opacity))
            }
        }
        .animation(.snappy(duration: 0.2), value: talkVisible)
        .refreshable {
            async let stats: Void = store.refreshStats()
            try? await store.refresh()
            await stats
        }
        .navigationTitle("Loft")
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                NavigationLink(value: SettingsRoute()) {
                    Image(systemName: "gearshape")
                }
                .accessibilityLabel("Settings")
                .accessibilityIdentifier("settingsButton")
            }
        }
        .createMenu()
    }

    @ViewBuilder private var activeWork: some View {
        SectionHeader("Active work")
        if !store.hasLoaded {
            Card {
                Label(store.connection.isReconnecting ? "Waiting for Loft…" : "Loading…", systemImage: "hourglass")
                    .font(.subheadline).foregroundStyle(.secondary)
            }
        } else if working.isEmpty {
            Card {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: "moon.stars")
                        .font(.title2)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("All quiet").font(.subheadline.weight(.semibold))
                        Text(store.agents.isEmpty ? "No agents are registered yet. Create one in Loft on your Mac."
                             : "No agent is working right now.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
                if !store.agents.isEmpty {
                    HStack {
                        if store.supportsTasks {
                            Button { router.composer = .newTask(agentId: nil, parentId: nil) } label: {
                                Label("New task", systemImage: "checklist")
                            }
                            .buttonStyle(.borderedProminent)
                        }
                        Button {
                            if let o = store.orchestrator { router.openChat(o.id) } else { router.composer = .pickRecipient }
                        } label: {
                            Label(store.orchestrator.map { "Message \($0.name)" } ?? "Start a conversation", systemImage: "bubble.left")
                        }
                        .buttonStyle(.bordered)
                    }
                    .controlSize(.small)
                }
            }
        } else {
            ForEach(working) { a in
                NavigationLink(value: AgentRoute(id: a.id)) {
                    WorkingAgentCard(agent: a)
                }
                .buttonStyle(.plain)
            }
        }
    }

    @ViewBuilder private var recentActivity: some View {
        SectionHeader("Recent activity", trailing: store.activity.isEmpty ? nil : AnyView(
            NavigationLink("See all", value: ActivityRoute()).font(.subheadline)
        ))
        if !store.supportsActivity && store.info != nil {
            Card {
                Label("This Loft server doesn't report activity yet. Update it on the Mac to see a feed here.", systemImage: "info.circle")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        } else if store.activity.isEmpty {
            Card {
                Text(store.hasLoaded ? "No recent activity." : store.connection.isReconnecting ? "Unavailable until Loft is reachable." : "Loading…").font(.subheadline).foregroundStyle(.secondary)
            }
        } else {
            VStack(spacing: 0) {
                ForEach(store.activity.prefix(8)) { item in
                    ActivityRow(item: item)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                    if item.id != store.activity.prefix(8).last?.id { Divider().padding(.leading, 44) }
                }
            }
            .background(Theme.card, in: RoundedRectangle(cornerRadius: Theme.radius, style: .continuous))
        }
    }
}

/// Floating shortcut to a voice conversation: with the orchestrator, or with an agent you pick
/// when none is set. Nothing is sent until you review it and tap Send.
private extension View {
    /// Sets `visible` to false while the scroll view moves down through its middle, and back to
    /// true on scrolling up or near the top or bottom (the bottom padding keeps the last rows clear).
    /// iOS 17 has no scroll geometry API, so there the button just stays put.
    @ViewBuilder func hidesOnScrollDown(_ visible: Binding<Bool>) -> some View {
        if #available(iOS 18, *) {
            onScrollGeometryChange(for: ScrollSpot.self) { g in
                ScrollSpot(offset: g.contentOffset.y + g.contentInsets.top,
                           remaining: g.contentSize.height + g.contentInsets.bottom - g.contentOffset.y - g.containerSize.height)
            } action: { old, new in
                let delta = new.offset - old.offset
                if new.offset < 40 || new.remaining < 40 { visible.wrappedValue = true }
                else if delta > 6 { visible.wrappedValue = false }
                else if delta < -6 { visible.wrappedValue = true }
            }
        } else {
            self
        }
    }
}

private struct ScrollSpot: Equatable {
    var offset: CGFloat
    var remaining: CGFloat
}

private struct TalkButton: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @ScaledMetric(relativeTo: .title2) private var size: CGFloat = 60
    @State private var picking = false
    /// Opens the full-screen voice conversation with this agent.
    let startVoice: (String) -> Void

    var body: some View {
        Button {
            // Voice style E starts with whoever you talked to last.
            if VoiceVariant.selected == .e, let id = UserDefaults.standard.string(forKey: "voiceLastAgent"), store.agent(id) != nil {
                startVoice(id)
            } else if let o = store.orchestrator { startVoice(o.id) } else { picking = true }
        } label: {
            Image(systemName: "mic.fill")
                .font(.title2.weight(.semibold))
                .foregroundStyle(.white)
                .frame(width: min(size, 88), height: min(size, 88))
                .background(Theme.accent, in: Circle())
                .shadow(color: .black.opacity(0.3), radius: 8, y: 4)
        }
        .buttonStyle(.plain)
        .contextMenu {
            if let o = store.orchestrator {
                Button { startVoice(o.id) } label: { Label("Voice conversation", systemImage: "waveform") }
                Button { router.openChat(o.id, dictate: true) } label: { Label("Dictate in chat", systemImage: "mic") }
            }
            Button { picking = true } label: { Label("Talk to another agent…", systemImage: "person.wave.2") }
            Button { router.composer = .pickRecipient } label: { Label("Message an agent", systemImage: "bubble.left") }
        }
        .sheet(isPresented: $picking) {
            NavigationStack {
                VoiceAgentPicker { id in
                    picking = false
                    startVoice(id)
                }
            }
            .presentationDetents([.medium, .large])
        }
        .accessibilityLabel(store.orchestrator.map { "Talk to \($0.name)" } ?? "Talk to an agent")
        .accessibilityHint(store.orchestrator == nil ? "Pick an agent, then start a voice conversation." : "Starts a voice conversation. You review the message before it's sent.")
        .accessibilityIdentifier("talkButton")
    }
}

/// Who to talk to, when there's no orchestrator (or you want someone else).
private struct VoiceAgentPicker: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let pick: (String) -> Void

    var body: some View {
        List {
            Section {
                ForEach(store.sortedAgents) { a in
                    Button { pick(a.id) } label: { AgentRow(agent: a).contentShape(Rectangle()) }
                        .buttonStyle(.plain)
                }
            } footer: {
                if store.orchestrator == nil {
                    Text("Choose an orchestrator in Settings and the Talk button goes straight to it.")
                }
            }
        }
        .navigationTitle("Talk to…")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        .accessibilityIdentifier("voiceAgentPicker")
    }
}

private struct ConnectionHeader: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        // At accessibility sizes the temperature goes on its own line, so it doesn't squeeze the status.
        AdaptiveStack(spacing: 8) {
            AdaptiveStack(spacing: 4) {
                HStack(spacing: 8) {
                    Circle()
                        .fill(store.connection.color)
                        .frame(width: 8, height: 8)
                    Text(store.connection.label)
                        .font(.subheadline.weight(.medium))
                    if !typeSize.isAccessibilitySize { Text("·").foregroundStyle(.tertiary) }
                }
                Text(store.info?.host ?? store.api?.baseURL.host() ?? "")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("connectionStatus")
            if !typeSize.isAccessibilitySize { Spacer(minLength: 4) }
            if let temp = store.system?.cpuTempC {
                TemperatureChip(celsius: temp, level: store.system?.level)
            }
        }
    }
}

/// The Mac's CPU temperature, like the web header: amber when warm (≥ 85 °C), red when hot (≥ 95 °C).
private struct TemperatureChip: View {
    let celsius: Double
    let level: String?

    private var color: Color {
        switch level {
        case "hot": Theme.failure
        case "warm": Theme.attention
        default: .secondary
        }
    }

    var body: some View {
        let degrees = Int(celsius.rounded())
        Label("\(degrees)°", systemImage: level == "hot" ? "thermometer.high" : level == "warm" ? "thermometer.medium" : "thermometer.low")
            .font(.caption.weight(.semibold).monospacedDigit())
            .fixedSize()
            .foregroundStyle(color)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(color.opacity(0.14), in: Capsule())
            .contentTransition(.numericText())
            .animation(.default, value: degrees)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Mac CPU temperature \(degrees) degrees" + (level == "hot" ? ", hot" : level == "warm" ? ", warm" : ""))
            .accessibilityIdentifier("macTemperature")
    }
}

private struct SummaryGrid: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let counts = Dictionary(grouping: store.agents, by: { store.state(of: $0) }).mapValues(\.count)
        // One column at accessibility sizes: two would break "Working" and "Approval" mid-word.
        let columns = Array(repeating: GridItem(.flexible(), spacing: 10), count: typeSize.isAccessibilitySize ? 1 : 2)
        LazyVGrid(columns: columns, spacing: 10) {
            tile(.working, counts[.working] ?? 0)
            tile(.waitingApproval, counts[.waitingApproval] ?? 0)
            tile(.idle, counts[.idle] ?? 0)
            tile(.failed, counts[.failed] ?? 0)
        }
    }

    private func tile(_ s: AgentState, _ n: Int) -> some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 2) {
                // "–" until something has loaded: a 0 would read as "no agents are working".
                Text(store.hasLoaded ? "\(n)" : "–")
                    .font(.system(.title, design: .rounded).weight(.semibold))
                    .contentTransition(.numericText())
                    .foregroundStyle(n > 0 ? s.color : .secondary)
                Text(s.shortLabel)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Image(systemName: s.symbol).foregroundStyle(n > 0 ? s.color : Color.secondary.opacity(0.5))
        }
        .padding(12)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: Theme.radius, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(store.hasLoaded ? "\(n) \(s.label)" : "\(s.label), not loaded yet")
        .animation(.default, value: n)
    }
}

/// Claude plan usage: the 5-hour session and the week, like the web meter. Hidden when the server doesn't report it.
private struct PlanUsageCard: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        if let usage = store.usage {
            Card {
                HStack {
                    Label("Claude plan", systemImage: "gauge.with.dots.needle.33percent")
                        .font(.subheadline.weight(.semibold))
                    Spacer()
                }
                if let error = usage.error {
                    Label("Usage unavailable", systemImage: "exclamationmark.circle")
                        .font(.footnote).foregroundStyle(.secondary)
                        .accessibilityHint(error)
                } else {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 16) { meters(usage) }
                        VStack(alignment: .leading, spacing: 14) { meters(usage) }
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("planUsageCard")
        }
    }

    @ViewBuilder private func meters(_ usage: PlanUsage) -> some View {
        if let s = usage.session { UsageMeter(title: "Session", window: s).frame(minWidth: 140) }
        if let w = usage.week { UsageMeter(title: "Week", window: w).frame(minWidth: 140) }
    }
}

/// Runs per day across all agents for two weeks, with totals underneath.
private struct ActivityCard: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        if !store.summaries.isEmpty {
            let t = store.totals
            let days = ActivityDay.build(auto: t.daily.map(\.auto), user: t.daily.map(\.user))
            SectionHeader("Last \(days.count) days")
            Card {
                ActivityChart(days: days)
                Divider()
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 0) { totals(t) }
                    VStack(alignment: .leading, spacing: 8) { totals(t) }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("activityCard")
        }
    }

    @ViewBuilder private func totals(_ t: (runs: Int, toolCalls: Int, prsMerged: Int, prsOpen: Int, daily: [(auto: Int, user: Int)])) -> some View {
        stat("\(t.runs)", "runs")
        Spacer(minLength: 8)
        stat("\(t.toolCalls)", "tool calls")
        Spacer(minLength: 8)
        stat("\(t.prsMerged) / \(t.prsOpen)", "PRs merged / open")
    }

    private func stat(_ value: String, _ label: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(value).font(.system(.headline, design: .rounded)).monospacedDigit()
            Text(label).font(.caption).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }
}

/// Everything waiting on the user: approvals, failures, blocked work.
private struct AttentionSection: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        let approvals = store.pendingApprovals()
        let failedAgents = store.agents.filter { store.state(of: $0) == .failed }
        let problemTasks = store.tasks.filter(\.state.needsAttention).filter { $0.state != .waitingApproval || approvals.isEmpty }
        if !approvals.isEmpty || !failedAgents.isEmpty || !problemTasks.isEmpty {
            SectionHeader("Needs your attention")
            VStack(spacing: 8) {
                ForEach(approvals) { p in ApprovalCard(request: p) }
                ForEach(problemTasks) { t in
                    NavigationLink(value: TaskRoute(id: t.id)) {
                        AttentionRow(symbol: t.state.symbol, color: t.state.color, title: t.title,
                                     detail: [t.state.label, t.error.isEmpty ? nil : t.error].compactMap { $0 }.joined(separator: " · "))
                    }
                    .buttonStyle(.plain)
                }
                ForEach(failedAgents) { a in
                    NavigationLink(value: AgentRoute(id: a.id)) {
                        AttentionRow(symbol: "exclamationmark.triangle.fill", color: Theme.failure, title: a.name,
                                     detail: (a.statusText?.isEmpty == false ? a.statusText! : "Last run failed"))
                    }
                    .buttonStyle(.plain)
                }
            }
            .accessibilityIdentifier("attentionSection")
        }
    }
}

private struct AttentionRow: View {
    let symbol: String
    let color: Color
    let title: String
    let detail: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol).foregroundStyle(color).frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.subheadline.weight(.semibold)).lineLimit(1)
                Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            }
            Spacer()
            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .padding(12)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: Theme.radius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius, style: .continuous).stroke(color.opacity(0.35), lineWidth: 1))
    }
}

/// A pending permission request, approvable in place.
struct ApprovalCard: View {
    @Environment(LoftStore.self) private var store
    let request: PermissionRequest
    @State private var busy = false
    @State private var error: APIError?
    @State private var confirmAlways = false
    /// The on-device model's plain sentence, once asked for; "" when it had none.
    @State private var explanation: String?
    @State private var explaining = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: "hand.raised.fill").foregroundStyle(Theme.attention)
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(store.agent(request.agentId)?.name ?? "An agent") needs approval")
                        .font(.subheadline.weight(.semibold))
                    RelativeTime(date: request.createdAt).font(.caption).foregroundStyle(.secondary)
                }
            }
            Text(request.summary)
                .font(.footnote.monospaced())
                .lineLimit(4)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
            if let reason = request.reason, !reason.isEmpty {
                Text(reason).font(.footnote).foregroundStyle(.secondary)
            }
            let risks = request.risks
            if !risks.isEmpty {
                // Always shown, model or not: what it could do that you'd want to know first.
                // In a row when they fit, else one under the other; each label at its natural size.
                // The accessibility element is a plain container around ViewThatFits, not ViewThatFits itself,
                // so it's one element with this label and identifier whichever layout fits.
                HStack(spacing: 0) {
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 6) { riskLabels(risks) }
                        VStack(alignment: .leading, spacing: 6) { riskLabels(risks) }
                    }
                    Spacer(minLength: 0)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("Warnings: " + risks.map(\.label).joined(separator: ", "))
                .accessibilityIdentifier("permissionRisks")
            }
            explainer
            AdaptiveStack {
                Button(role: .destructive) { decide("deny") } label: { Text("Deny").frame(maxWidth: .infinity) }
                    .buttonStyle(.bordered)
                Button { decide("allow_once") } label: { Text("Allow once").frame(maxWidth: .infinity) }
                    .buttonStyle(.borderedProminent)
                Menu {
                    Button("Always allow \(request.rule ?? request.toolName)") { confirmAlways = true }
                } label: {
                    Image(systemName: "ellipsis").frame(height: 20)
                }
                .buttonStyle(.bordered)
                .accessibilityLabel("More options")
            }
            .disabled(busy || !store.canWrite)
            .controlSize(.small)
        }
        .padding(12)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: Theme.radius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius, style: .continuous).stroke(Theme.attention.opacity(0.4), lineWidth: 1))
        .confirmationDialog("Always allow this?", isPresented: $confirmAlways, titleVisibility: .visible) {
            Button("Always allow") { decide("allow_always") }
        } message: {
            Text("\(request.rule ?? request.toolName) is added to the agent's allowed tools, so it won't ask again.")
        }
        .errorAlert($error)
        .sensoryFeedback(.success, trigger: busy) { old, new in old && !new && error == nil }
        // A container with its own identifier: on a plain stack it would also replace the identifiers inside
        // (permissionRisks, permissionExplain, permissionExplanation).
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("approvalCard")
    }

    private func riskLabels(_ risks: [PermissionRisk]) -> some View {
        ForEach(risks) { r in
            // An explicit icon and text, not a Label: inside the card's layout the Label drew only its icon.
            HStack(spacing: 4) {
                Image(systemName: r.symbol)
                Text(r.label).lineLimit(1).fixedSize()
            }
                .font(.caption.weight(.semibold))
                .foregroundStyle(r.serious ? Theme.failureInk : Theme.attentionInk)
                .padding(.horizontal, 8).padding(.vertical, 3)
                .background((r.serious ? Theme.failure : Theme.attention).opacity(0.12), in: Capsule())
        }
    }

    /// "What does this do?", then the model's sentence (labelled as a suggestion), or a note that there isn't one.
    @ViewBuilder private var explainer: some View {
        if let e = explanation {
            if e.isEmpty {
                Text("No plain explanation for this one. Check the request above.").font(.footnote).foregroundStyle(.secondary)
            } else {
                Label(e, systemImage: "sparkles").font(.footnote)
                    .accessibilityLabel("Plain explanation, from the on-device model: \(e)")
                    .accessibilityIdentifier("permissionExplanation")
            }
        } else if explaining {
            Label("Explaining…", systemImage: "sparkles").font(.footnote).foregroundStyle(.secondary)
        } else if OnDeviceFeatures.explainOn {
            Button { explain() } label: { Label("What does this do?", systemImage: "sparkles") }
                .font(.footnote).frame(minHeight: 44)
                .buttonStyle(.borderless)
                .accessibilityIdentifier("permissionExplain")
        }
    }

    private func explain() {
        explaining = true
        VoiceLog.shared.add("ai: explanation asked for \(request.toolName)")
        Task {
            explanation = await OnDeviceFeatures.explain(request) ?? ""
            explaining = false
        }
    }

    private func decide(_ d: String) {
        busy = true
        Task {
            do { try await store.decide(request, decision: d) } catch { self.error = APIError.from(error) }
            busy = false
        }
    }
}

private struct WorkingAgentCard: View {
    @Environment(LoftStore.self) private var store
    let agent: Agent

    var body: some View {
        let state = store.state(of: agent)
        let task = store.currentTask(for: agent.id)
        Card {
            AdaptiveStack(spacing: 10) {
                HStack(spacing: 10) {
                    AgentAvatar(name: agent.name, id: agent.id, size: 34, state: state)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(agent.name).font(.subheadline.weight(.semibold))
                        if let p = agent.project, !p.isEmpty {
                            Text(p).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer(minLength: 0)
                }
                ChipRow {
                    if let ctx = agent.contextTokens { ContextChip(tokens: ctx) }
                    StatusBadge(state, compact: true)
                }
            }
            if let task {
                Label(task.title, systemImage: "checklist")
                    .font(.footnote.weight(.medium))
                    .lineLimit(1)
            }
            if let text = agent.statusText, !text.isEmpty {
                Text(text)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            if let started = agent.lastRunAt, state == .working {
                TimelineView(.periodic(from: .now, by: 30)) { ctx in
                    Label("Running for \(started.durationSince(ctx.date))", systemImage: "clock")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityHint("Opens agent details")
    }
}

struct ActivityRow: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    @Environment(\.dynamicTypeSize) private var typeSize
    /// Grows with the text so a large icon doesn't spill onto the words next to it.
    @ScaledMetric(relativeTo: .subheadline) private var iconWidth: CGFloat = 22
    let item: ActivityItem

    private var symbol: String {
        switch item.type {
        case "agent.started": "play.circle"
        case "agent.completed": "checkmark.circle"
        case "agent.stopped": "stop.circle"
        case "agent.failed", "task.failed": "exclamationmark.triangle"
        case "approval.required": "hand.raised"
        case "agent.message": "arrow.left.arrow.right"
        case "task.created": "plus.circle"
        case "task.completed": "checkmark.seal"
        case "task.blocked": "pause.circle"
        case "task.cancelled": "minus.circle"
        default: "circle"
        }
    }

    private var color: Color {
        switch item.severity {
        case "error": Theme.failure
        case "attention": Theme.attention
        case "success": Theme.working
        default: .secondary
        }
    }

    var body: some View {
        Button(action: open) {
            Group {
                if typeSize.isAccessibilitySize {
                    // Icon and time on one line, the summary under it at full width and never cut short:
                    // side by side, the summary got a few letters per line ("Orches-/trator cr…").
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            icon
                            Spacer(minLength: 8)
                            time
                        }
                        summary.lineLimit(nil)
                    }
                } else {
                    HStack(alignment: .top, spacing: 10) {
                        icon.frame(width: iconWidth)
                        summary.lineLimit(2)
                        time
                    }
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(item.summary), \(item.at.formatted(date: .abbreviated, time: .shortened))")
    }

    private var icon: some View { Image(systemName: symbol).foregroundStyle(color) }

    private var summary: some View {
        Text(item.summary)
            .font(.subheadline)
            .foregroundStyle(.primary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var time: some View {
        RelativeTime(date: item.at)
            .font(.caption)
            .foregroundStyle(.secondary)
            .monospacedDigit()
    }

    private func open() {
        if let t = item.taskId, store.task(t) != nil {
            push(TaskRoute(id: t))
        } else if let a = item.agentId, store.agent(a) != nil {
            if item.messageId != nil { router.openChat(a) } else { push(AgentRoute(id: a)) }
        }
    }

    private func push<R: Hashable>(_ r: R) {
        switch router.tab {
        case .overview: router.overviewPath.append(r)
        case .agents: router.agentsPath.append(r)
        case .tasks: router.tasksPath.append(r)
        case .chat: router.chatPath.append(r)
        }
    }
}

struct ActivityListView: View {
    @Environment(LoftStore.self) private var store

    var body: some View {
        List {
            ForEach(store.activity) { item in
                ActivityRow(item: item)
            }
            if store.activityHasMore {
                ProgressView()
                    .frame(maxWidth: .infinity)
                    .task { await store.loadMoreActivity() }
            }
        }
        .overlay {
            if store.activity.isEmpty {
                EmptyState(symbol: "clock", title: "No activity", message: "Runs, approvals and task updates show up here.")
            }
        }
        .refreshable { try? await store.refresh() }
        .navigationTitle("Activity")
    }
}
