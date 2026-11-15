import LoftKit
import SwiftUI

/// The Agents section: Leads first, then each group, beside the chosen agent's chat and its settings.
struct AgentsSection: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    @State private var search = ""
    @State private var showsSettings = false

    private var filtered: [Agent] {
        store.sortedAgents.filter { search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) }
    }

    var body: some View {
        @Bindable var nav = nav
        HSplitView {
            VStack(spacing: 0) {
                AgentSearchField(text: $search)
                    .padding(.horizontal, 10).padding(.top, 10).padding(.bottom, 4)
                // A scroll view of rows, as the web's list: macOS's List kept these rows at its own default height.
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 2) {
                            ForEach(AgentSection.sections(filtered)) { section in
                                Text(section.title).eyebrow().accessibilityLabel(section.title)
                                    .padding(.horizontal, 10).padding(.top, 12).padding(.bottom, 4)
                                ForEach(section.agents) { a in
                                    AgentListRow(agent: a, showsGroup: section.id == "leads", selected: a.id == nav.agentId) {
                                        nav.agentId = a.id
                                    }
                                    .id(a.id)
                                }
                            }
                        }
                        .padding(.horizontal, 8).padding(.bottom, 12)
                    }
                    .accessibilityIdentifier("agentsList")
                    .focusable()
                    .focusEffectDisabled()
                    .onMoveCommand { move($0) }
                    .onChange(of: nav.agentId) { _, id in if let id { proxy.scrollTo(id) } }
                }
                .overlay {
                    if store.lastSync != nil && store.agents.isEmpty {
                        ContentUnavailableView("No agents", systemImage: "person.3", description: Text("Create agents in Loft first."))
                    } else if filtered.isEmpty && !search.isEmpty {
                        ContentUnavailableView.search(text: search)
                    }
                }
            }
            .frame(minWidth: 240, idealWidth: 280, maxWidth: 380)

            Group {
                if let agent = store.agent(nav.agentId) {
                    ChatView(agent: agent, showsSettings: $showsSettings)
                        .id(agent.id)
                        .inspector(isPresented: $showsSettings) {
                            AgentSettingsView(agent: agent).id(agent.id)
                        }
                } else {
                    ContentUnavailableView("Pick an agent", systemImage: "bubble.left.and.bubble.right",
                                           description: Text("Its chat opens here. ⌘N starts a new message."))
                }
            }
            .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
        }
        .navigationTitle(store.agent(nav.agentId)?.name ?? "Agents")
        .onAppear {
            // Opens on the Lead's chat the first time.
            if nav.agentId == nil { nav.agentId = store.orchestrator?.id ?? AgentSection.sections(store.agents).first?.agents.first?.id }
        }
    }

    /// ↑ and ↓ pick the agent before or after the chosen one, in the list's order.
    private func move(_ direction: MoveCommandDirection) {
        let ids = AgentSection.sections(filtered).flatMap(\.agents).map(\.id)
        guard !ids.isEmpty, direction == .up || direction == .down else { return }
        let i = ids.firstIndex { $0 == nav.agentId }
        let next = i.map { direction == .up ? $0 - 1 : $0 + 1 } ?? 0
        if ids.indices.contains(next) { nav.agentId = ids[next] }
    }
}

/// A row of the Agents list: the web's row background when selected or under the pointer.
struct AgentListRow: View {
    let agent: Agent
    var showsGroup = false
    var selected = false
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            AgentRow(agent: agent, showsGroup: showsGroup)
                .padding(.horizontal, 10)
                .background(WebTheme.fg.opacity(selected ? 0.06 : hovering ? 0.03 : 0),
                            in: RoundedRectangle(cornerRadius: WebTheme.rowRadius, style: .continuous))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("agentRow-\(agent.name)")
    }
}

/// The web's agents list row: the agent's sticker with its status dot, its name, "project · role" in mono, and its
/// status: the state's label, then what it says it's doing.
struct AgentRow: View {
    let agent: Agent
    /// Shows its group before "project · role" (in Leads, where the section title isn't the group).
    var showsGroup = false

    var body: some View {
        let status = QuickSearch.AgentStatus(agent)
        HStack(spacing: 12) {
            AgentSticker(agent: agent, size: 40)
                .overlay(alignment: .bottomTrailing) {
                    Circle().fill(status.dot).frame(width: 12, height: 12)
                        .overlay(Circle().strokeBorder(WebTheme.canvas, lineWidth: 2))
                        .offset(x: 2, y: 2)
                }
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    Text(agent.name).font(.system(size: 13.5)).foregroundStyle(WebTheme.fg).lineLimit(1).truncationMode(.tail)
                    Spacer(minLength: 0)
                    // As the web's list: the Session's size as of its last reply, amber past the smart zone.
                    if let ctx = AgentHeader.listedContext(agent) {
                        Text(AgentHeader.tokens(ctx))
                            .font(.system(size: 10.5, design: .monospaced)).monospacedDigit()
                            .foregroundStyle(ContextWindow.warns(ctx) ? Theme.attentionInk : WebTheme.fg3)
                            .help("\(ctx.formatted()) tokens in context as of the last reply")
                            .accessibilityLabel("Context \(AgentHeader.tokens(ctx)) tokens" + (ContextWindow.warns(ctx) ? ", past the smart zone" : ""))
                            .accessibilityIdentifier("agentContext-\(agent.name)")
                    }
                }
                if !detail.isEmpty {
                    Text(detail).font(.system(size: 10.5, design: .monospaced)).foregroundStyle(WebTheme.fg3).lineLimit(1)
                }
                (Text(status.label).foregroundStyle(status.text)
                    + Text(agent.statusText.map { $0.isEmpty ? "" : " · \($0)" } ?? "").foregroundStyle(WebTheme.fg3))
                    .font(.system(size: 10.5, design: .monospaced)).lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
        .accessibilityValue(status.label)
        .accessibilityIdentifier("agentRow-\(agent.name)")
    }

    private var detail: String {
        let line = agent.projectAndRole
        guard showsGroup, !agent.team.isEmpty, agent.team.caseInsensitiveCompare(agent.project ?? "") != .orderedSame else { return line }
        return line.isEmpty ? agent.team : "\(agent.team) · \(line)"
    }
}

/// The web's search field over a list: a magnifier and the text, in a rounded hairline box.
struct AgentSearchField: View {
    @Binding var text: String

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").font(.system(size: 12)).foregroundStyle(WebTheme.fg3).accessibilityHidden(true)
            TextField("Search agents", text: $text)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .accessibilityIdentifier("agentSearch")
            if !text.isEmpty {
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(WebTheme.fg3) }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 7)
        .background(WebTheme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(WebTheme.line))
    }
}

/// An agent's sticker, as on the web, drawn off the main actor the first time; its initials until then, or without one.
struct AgentSticker: View {
    let agent: Agent
    var size: CGFloat = 28
    @Environment(\.displayScale) private var scale
    @State private var image: NSImage?

    private static let cache = NSCache<NSString, NSImage>()

    var body: some View {
        let svg = agent.avatar?.svg
        let pixels = Int(size * scale), lead = agent.isLead
        let key = "\(svg?.hashValue ?? 0)@\(pixels)\(lead ? "+lead" : "")" as NSString
        Group {
            if let image = image ?? Self.cache.object(forKey: key) {
                Image(nsImage: image).resizable().frame(width: size, height: size)
            } else {
                Initials(name: agent.name, lead: lead, size: size * 0.8).frame(width: size, height: size)
            }
        }
        .accessibilityHidden(true)
        .task(id: key) {
            guard let svg, Self.cache.object(forKey: key) == nil else { return }
            let data = await Task.detached(priority: .userInitiated) { Sticker.png(svg: svg, pixels: pixels, lead: lead) }.value
            guard let data, let drawn = NSImage(data: data) else { return }
            Self.cache.setObject(drawn, forKey: key)
            image = drawn
        }
    }
}

/// A round badge with the agent's initials (a Lead's is filled).
struct Initials: View {
    let name: String
    var lead = false
    var size: CGFloat = 28

    var body: some View {
        let letters = name.split(separator: " ").prefix(2).compactMap(\.first).map(String.init).joined()
        Text(letters)
            .font(.system(size: size * 0.4, weight: .bold))
            .foregroundStyle(lead ? Theme.onAccent : Theme.accent)
            .frame(width: size, height: size)
            .background(lead ? Theme.accent : Theme.accent.opacity(0.15), in: Circle())
            .accessibilityHidden(true)
    }
}

/// An agent's settings, as on the web: saved together, only what changed.
struct AgentSettingsView: View {
    @Environment(LoftStore.self) private var store
    let agent: Agent
    @State private var original: AgentDraft
    @State private var draft: AgentDraft
    @State private var saving = false
    @State private var error: String?

    init(agent: Agent) {
        self.agent = agent
        _original = State(initialValue: AgentDraft(agent))
        _draft = State(initialValue: AgentDraft(agent))
    }

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $draft.name).accessibilityIdentifier("settingsName")
                Picker("Role", selection: $draft.role) {
                    ForEach(Agent.roles, id: \.id) { Text($0.label).tag($0.id) }
                }
                TextField("Group", text: $draft.group, prompt: Text("No group"))
                    .help("Its team: agents in the same group are listed together and share one Lead.")
            }
            Section("Standing instructions") {
                TextEditor(text: $draft.prompt)
                    .font(.body)
                    .frame(minHeight: 120)
                    .accessibilityIdentifier("settingsPrompt")
            }
            Section {
                Picker("Model", selection: $draft.model) {
                    ForEach(choices(AgentDraft.models, current: draft.model), id: \.0) { Text($0.1).tag($0.0) }
                }
                Picker("Effort", selection: $draft.effort) {
                    ForEach(AgentDraft.efforts, id: \.0) { Text($0.1).tag($0.0) }
                }
                Picker("Permission mode", selection: $draft.permissionMode) {
                    ForEach(choices(AgentDraft.permissionModes, current: draft.permissionMode), id: \.0) { Text($0.1).tag($0.0) }
                }
                Picker("Autonomy", selection: $draft.intervalMinutes) {
                    ForEach(intervals, id: \.0) { Text($0.1).tag($0.0) }
                }
                if !agent.isLead || agent.freshContextApplies != false {
                    Picker("Fresh context", selection: $draft.freshContext) {
                        ForEach(AgentDraft.freshContexts, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .help("Before Delivery starts it on a new ticket, Loft clears its context: always, or once it reaches the threshold.")
                }
            }
            Section {
                HStack {
                    Button(saving ? "Saving…" : "Save changes") { Task { await save() } }
                        .keyboardShortcut("s")
                        .disabled(draft == original || saving || !store.canWrite)
                        .accessibilityIdentifier("settingsSave")
                    if draft != original {
                        Button("Discard") { draft = original }
                    }
                }
                if let error {
                    Text(error).font(.footnote).foregroundStyle(Theme.failureInk)
                }
            }
        }
        .formStyle(.grouped)
        .inspectorColumnWidth(min: 280, ideal: 300, max: 420)
    }

    /// The listed choices, plus the current value when it isn't one of them (set elsewhere).
    private func choices(_ list: [(String, String)], current: String) -> [(String, String)] {
        list.contains { $0.0 == current } ? list : list + [(current, current.isEmpty ? "Default" : current)]
    }

    private var intervals: [(Int, String)] {
        AgentDraft.intervals.contains { $0.0 == draft.intervalMinutes }
            ? AgentDraft.intervals : AgentDraft.intervals + [(draft.intervalMinutes, "Every \(draft.intervalMinutes) min")]
    }

    private func save() async {
        saving = true
        defer { saving = false }
        do {
            try await store.updateAgent(agent.id, draft.changes(from: original))
            original = draft
            error = nil
        } catch {
            self.error = APIError.from(error).title
        }
    }
}
