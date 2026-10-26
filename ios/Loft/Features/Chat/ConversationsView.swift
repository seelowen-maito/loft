import SwiftUI

/// Every agent has one conversation in Loft. The orchestrator is the agent the
/// user picked to coordinate the others; it is a normal agent, labelled as such.
struct ConversationsView: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dynamicTypeSize) private var typeSize
    @State private var choosing = false

    var body: some View {
        Group {
            if !store.hasLoaded {
                VStack(spacing: 0) {
                    StaleBanner().padding(.horizontal, Theme.gutter)
                    NotLoadedView()
                }
            } else if store.agents.isEmpty {
                EmptyState(symbol: "bubble.left.and.bubble.right", title: "No conversations yet",
                           message: "Create an agent in Loft on your Mac to start talking to it.")
            } else {
                list
            }
        }
        .background(Theme.background)
        .navigationTitle("Chat")
        .sheet(isPresented: $choosing) {
            NavigationStack {
                AgentPickerList(title: "Orchestrator") { a in
                    store.orchestratorId = a.id
                    choosing = false
                }
            }
            .presentationDetents([.medium, .large])
        }
        .createMenu()
    }

    private var list: some View {
        List {
            StaleBanner().listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
            Section {
                if let o = store.orchestrator {
                    NavigationLink(value: ChatRoute(agentId: o.id)) {
                        HStack(spacing: 12) {
                            if !typeSize.isAccessibilitySize {
                                AgentAvatar(name: o.name, id: o.id, size: 44, state: store.state(of: o))
                            }
                            VStack(alignment: .leading, spacing: 3) {
                                if typeSize.isAccessibilitySize {
                                    // At accessibility sizes the name gets its own line and shrinks rather than
                                    // breaking mid-word ("Orchestra-tor"); the sparkle goes on the line below.
                                    Text(o.name).font(.headline).lineLimit(1).minimumScaleFactor(0.5)
                                    Image(systemName: "sparkles").foregroundStyle(Theme.accent).font(.caption)
                                        .accessibilityHidden(true)
                                } else {
                                    (Text(o.name) + Text(" ") + Text(Image(systemName: "sparkles")).foregroundStyle(Theme.accent).font(.caption))
                                        .font(.headline)
                                }
                                Text(preview(o))
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                                    .readableLineLimit(2)
                            }
                        }
                        .padding(.vertical, 4)
                    }
                    .accessibilityIdentifier("orchestratorConversation")
                } else {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("Pick an orchestrator", systemImage: "sparkles").font(.headline)
                        Text("Loft doesn't have a built-in orchestrator. Choose the agent you want to plan work and hand it to the others. It can delegate with Loft's task and messaging tools.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                        Button("Choose agent") { choosing = true }
                            .buttonStyle(.borderedProminent)
                            .disabled(store.agents.isEmpty)
                    }
                    .padding(.vertical, 6)
                }
            } header: {
                Text("Orchestrator")
            } footer: {
                if store.orchestrator != nil {
                    Button { choosing = true } label: {
                        Text("Change orchestrator")
                            .multilineTextAlignment(.leading)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .font(.footnote)
                }
            }

            Section("Direct with an agent") {
                ForEach(store.sortedAgents.filter { $0.id != store.orchestratorId }) { a in
                    NavigationLink(value: ChatRoute(agentId: a.id)) {
                        HStack(spacing: 12) {
                            if !typeSize.isAccessibilitySize {
                                AgentAvatar(name: a.name, id: a.id, size: 36, state: store.state(of: a))
                            }
                            VStack(alignment: .leading, spacing: 2) {
                                Text(a.name).font(.body.weight(.medium))
                                Text(preview(a)).font(.footnote).foregroundStyle(.secondary).readableLineLimit(1)
                            }
                            Spacer()
                            if let unsent = Optional(store.pending(for: a.id).filter(\.isFailed).count), unsent > 0 {
                                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.failure)
                                    .accessibilityLabel("\(unsent) unsent")
                            }
                        }
                    }
                }
            }
        }
        .refreshable { try? await store.refresh() }
    }

    private func preview(_ a: Agent) -> String {
        if let t = a.statusText, !t.isEmpty { return t }
        if let last = store.conversation(a.id).last(where: { $0.kind == "text" }) {
            return (last.role == "user" ? "You: " : "") + last.content.replacingOccurrences(of: "\n", with: " ")
        }
        if let t = a.title, !t.isEmpty { return t }
        if let last = a.lastRunAt { return "Last active \(last.shortAgo == "now" ? "just now" : last.shortAgo + " ago")" }
        return "Start a conversation"
    }
}
