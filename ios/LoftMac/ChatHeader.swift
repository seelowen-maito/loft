import LoftKit
import SwiftUI

/// The chat's header, as on the web: the agent's sticker and name, its status, schedule and context size, then
/// Stop or Wake, Restart while it runs, Clear context and the settings toggle.
struct ChatHeader: View {
    @Environment(LoftStore.self) private var store
    let agent: Agent
    @Binding var showsSettings: Bool
    @State private var resuming = false
    @State private var error: String?

    private var running: Bool { agent.running == true }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 14) {
                AgentSticker(agent: agent, size: 44)
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(agent.name).font(.system(size: 16, weight: .medium)).foregroundStyle(WebTheme.fg).lineLimit(1)
                        Text(agent.roleLabel).font(.system(size: 12.5)).foregroundStyle(WebTheme.fg3).lineLimit(1)
                    }
                    .accessibilityElement(children: .combine)
                    HStack(spacing: 12) {
                        StatusLine(agent: agent)
                        // A minute is the schedule's smallest step: "next in 12m" moves with it.
                        TimelineView(.everyMinute) { t in
                            Label(AgentHeader.schedule(agent, now: t.date), systemImage: "clock")
                                .labelStyle(.titleAndIcon)
                                .font(.system(size: 11, design: .monospaced)).foregroundStyle(WebTheme.fg3)
                                .lineLimit(1)
                                .help(AgentHeader.quietHours(agent) ?? "")
                                .accessibilityLabel(["Schedule", AgentHeader.schedule(agent, now: t.date), AgentHeader.quietHours(agent)]
                                    .compactMap { $0 }.joined(separator: ", "))
                                .accessibilityIdentifier("chatSchedule")
                        }
                        contextMenu
                    }
                }
                Spacer(minLength: 8)
                actions
            }
            if let error {
                Text(error).font(.system(size: 11.5)).foregroundStyle(Theme.failureInk)
                    .accessibilityIdentifier("chatHeaderError")
            }
        }
        .padding(.horizontal, 20).padding(.vertical, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(WebTheme.line).frame(height: 1) }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatHeader")
        .sheet(isPresented: $resuming) { ResumeSheet(agentId: agent.id) }
    }

    private var actions: some View {
        HStack(spacing: 6) {
            if running {
                Button { act { try await store.stopRun(agent.id) } } label: { Label("Stop", systemImage: "stop.fill") }
                    .help("Stop")
                    .accessibilityIdentifier("chatStop")
                Button { act { try await store.restartRun(agent.id) } } label: { Label("Restart", systemImage: "arrow.clockwise") }
                    .help("Stops this run. Queued messages are kept and run next.")
                    .accessibilityIdentifier("chatRestart")
            } else {
                Button { act { try await store.runNow(agent.id) } } label: { Label("Wake", systemImage: "play.fill") }
                    .help("Wake: run an autonomous check-in now")
                    .accessibilityIdentifier("chatWake")
            }
            Button { act { try await store.resetSession(agent.id) } } label: {
                Label("Clear context", systemImage: "arrow.counterclockwise").labelStyle(.iconOnly)
            }
            .help("Clear context (/clear). Memory is kept.")
            .disabled(running)
            .accessibilityIdentifier("chatClearContext")
            Button { showsSettings.toggle() } label: {
                Label("Agent settings", systemImage: "slider.horizontal.3").labelStyle(.iconOnly)
            }
            .keyboardShortcut("i", modifiers: [.command, .option])
            .help("Show or hide this agent's settings (⌥⌘I)")
            .accessibilityIdentifier("agentSettingsButton")
        }
        .buttonStyle(.bordered)
        .controlSize(.small)
        .disabled(!store.canWrite)
    }

    /// The context size: the warning colour past the smart zone; it opens Compact and Clear for the Session, and
    /// Resume always (the only one with no Session), as the web's.
    private var contextMenu: some View {
        let session = AgentHeader.hasSession(agent)
        let warns = agent.contextTokens.map(ContextWindow.warns) ?? false
        return Menu {
            if running { Text("Stop the agent first.") }
            if session {
                Button { act { try await store.compact(agent.id) } } label: {
                    Label("Compact", systemImage: "arrow.down.right.and.arrow.up.left")
                    Text("Summarize the conversation so later runs carry less.")
                }
                .disabled(running)
                .accessibilityIdentifier("contextCompact")
                Button { act { try await store.resetSession(agent.id) } } label: {
                    Label("Clear", systemImage: "arrow.counterclockwise")
                    Text("Start a fresh Session. Memory is kept.")
                }
                .disabled(running)
                .accessibilityIdentifier("contextClear")
            }
            Button { resuming = true } label: {
                Label("Resume a session", systemImage: "clock.arrow.circlepath")
                Text("Go back to one of this agent's past sessions.")
            }
            .disabled(running)
            .accessibilityIdentifier("contextResume")
        } label: {
            Text(AgentHeader.context(agent))
                .font(.system(size: 11, design: .monospaced)).monospacedDigit()
                .foregroundStyle(warns ? Theme.attentionInk : WebTheme.fg3)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help(!session ? "No Session yet: the next run starts a fresh one, or resume a past one."
              : agent.contextTokens.map { "\($0.formatted()) tokens in context as of the last reply. Every turn resends it, so a big context uses your limits faster; Compact shrinks it." }
              ?? "Context size shows after the next reply.")
        .accessibilityLabel(!session ? "No session" : agent.contextTokens.map {
            "Context \(AgentHeader.tokens($0)) tokens" + (warns ? ", past the 80k smart zone" : "")
        } ?? "Context size not known yet")
        .accessibilityIdentifier(warns ? "contextSizeOver" : "contextSize")
    }

    private func act(_ action: @escaping () async throws -> Void) {
        error = nil
        Task { do { try await action() } catch { self.error = APIError.from(error).title } }
    }
}

/// The web's status pill: a dot, the state in upper case, and how long it has run.
private struct StatusLine: View {
    let agent: Agent

    var body: some View {
        let status = QuickSearch.AgentStatus(agent)
        HStack(spacing: 6) {
            Circle().fill(status.dot).frame(width: 6, height: 6)
            Text(status.label).textCase(.uppercase).tracking(1).foregroundStyle(status.text)
            if agent.running == true {
                TimelineView(.periodic(from: .now, by: 1)) { t in
                    Text("· \(RunTimer.elapsed(since: agent.lastRunAt, now: t.date))").foregroundStyle(WebTheme.fg3).monospacedDigit()
                }
            }
        }
        .font(.system(size: 11, design: .monospaced))
        .lineLimit(1)
        .fixedSize()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("chatStatus")
    }
}

/// "Resume a session": the agent's past Sessions, newest first; picking one makes it the agent's Session, as the
/// web's /resume does.
private struct ResumeSheet: View {
    @Environment(LoftStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let agentId: String
    @State private var list: PastSessions?
    @State private var error: String?
    @State private var busy: String?

    private var running: Bool { store.agent(agentId)?.running == true }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Resume a session").font(.headline)
            if running { Text("Stop the agent first.").foregroundStyle(Theme.attentionInk) }
            if let error { Text(error).foregroundStyle(Theme.failureInk) }
            if let list {
                if list.sessions.isEmpty { Text("No past Sessions for this agent yet.").foregroundStyle(WebTheme.fg3) }
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(list.sessions) { s in
                            let current = s.id == list.current
                            Button { resume(s.id) } label: { row(s, current: current) }
                                .buttonStyle(.plain)
                                .disabled(running || current || busy != nil)
                                .accessibilityIdentifier("pastSession-\(s.id)")
                        }
                    }
                }
            } else if error == nil {
                HStack(spacing: 8) { ProgressView().controlSize(.small); Text("Reading the transcripts…").foregroundStyle(WebTheme.fg3) }
            }
            Text("The agent's next run picks up where that Session left off. Its whole context goes back into the prompt cache at full price, so a big one costs more.")
                .font(.footnote).foregroundStyle(WebTheme.fg3).fixedSize(horizontal: false, vertical: true)
            HStack { Spacer(); Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction) }
        }
        .padding(20)
        .frame(width: 460)
        .frame(minHeight: 260)
        .task {
            do { list = try await store.pastSessions(agentId) } catch { self.error = APIError.from(error).title }
        }
    }

    private func row(_ s: PastSession, current: Bool) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            if busy == s.id { ProgressView().controlSize(.small) } else { Image(systemName: "clock.arrow.circlepath").foregroundStyle(WebTheme.fg3) }
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(s.startedAt.formatted(date: .abbreviated, time: .shortened))
                    if current { Text("current").foregroundStyle(Theme.accent) }
                    Spacer()
                    if let n = s.contextTokens {
                        Text("context \(AgentHeader.tokens(n))").monospacedDigit()
                            .foregroundStyle(ContextWindow.warns(n) ? Theme.attentionInk : WebTheme.fg3)
                    }
                }
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(WebTheme.fg3)
                (Text(s.fromAgent.map { "\($0): " } ?? "").foregroundStyle(WebTheme.fg3) + Text(s.firstMessage).foregroundStyle(WebTheme.fg))
                    .lineLimit(2)
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 8)
        .contentShape(Rectangle())
    }

    private func resume(_ id: String) {
        busy = id
        error = nil
        Task {
            do {
                try await store.resumeSession(agentId, sessionId: id)
                dismiss()
            } catch {
                self.error = APIError.from(error).title
                busy = nil
            }
        }
    }
}
