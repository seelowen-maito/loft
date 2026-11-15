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
            List(selection: $nav.agentId) {
                ForEach(AgentSection.sections(filtered)) { section in
                    Section(section.title) {
                        ForEach(section.agents) { a in
                            AgentRow(agent: a, showsGroup: section.id == "leads").tag(a.id)
                        }
                    }
                }
            }
            .searchable(text: $search, placement: .sidebar, prompt: "Search agents")
            .overlay {
                if store.lastSync != nil && store.agents.isEmpty {
                    ContentUnavailableView("No agents", systemImage: "person.3", description: Text("Create agents in Loft first."))
                }
            }
            .frame(minWidth: 220, idealWidth: 260, maxWidth: 380)
            .accessibilityIdentifier("agentsList")

            Group {
                if let agent = store.agent(nav.agentId) {
                    ChatView(agent: agent)
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
        .toolbar {
            if nav.agentId != nil {
                ToolbarItem {
                    Button { showsSettings.toggle() } label: { Label("Agent settings", systemImage: "slider.horizontal.3") }
                        .keyboardShortcut("i", modifiers: [.command, .option])
                        .help("Show or hide this agent's settings (⌥⌘I)")
                        .accessibilityIdentifier("agentSettingsButton")
                }
            }
        }
        .onAppear {
            // Opens on the Lead's chat the first time.
            if nav.agentId == nil { nav.agentId = store.orchestrator?.id ?? AgentSection.sections(store.agents).first?.agents.first?.id }
        }
    }
}

struct AgentRow: View {
    @Environment(LoftStore.self) private var store
    let agent: Agent
    /// Shows its group under the name (in Leads, where the section title isn't the group).
    var showsGroup = false

    var body: some View {
        let state = store.state(of: agent)
        HStack(spacing: 10) {
            Initials(name: agent.name, lead: agent.isLead)
            VStack(alignment: .leading, spacing: 2) {
                Text(agent.name).fontWeight(.semibold).lineLimit(1)
                if showsGroup, !agent.team.isEmpty {
                    Text(agent.team).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Text(subtitle(state)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 4)
            Circle().fill(state.color).frame(width: 8, height: 8).help(state.label).accessibilityHidden(true)
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityValue(state.label)
        .accessibilityIdentifier("agentRow-\(agent.name)")
    }

    private func subtitle(_ s: AgentState) -> String {
        if let t = agent.statusText, !t.isEmpty { return t }
        return s.label
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
