import LoftKit
import SwiftUI

/// One agent's conversation. Messages go straight to this agent through Loft's
/// message API; nothing is rerouted.
struct ChatView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Router.self) private var router
    let agentId: String
    @State private var draft: String
    @State private var loading = true
    @State private var loadError: APIError?
    @State private var loadingOlder = false
    /// The item that was at the top when you tapped "Load earlier messages": it goes back to the top once the
    /// earlier ones are drawn above it.
    @State private var keepTop: String?
    /// How many of the latest chat items are drawn; "Load earlier messages" shows more.
    @State private var shown = Self.page
    private static let page = 60
    @State private var confirmStop = false
    @State private var actionError: APIError?
    @State private var confirmEndGrill = false
    /// The open interview full screen over the chat (`InterviewView`).
    @State private var showingInterview = false
    // Grill voice loop, all inside this chat: read the question, open the mic, catch spoken commands.
    @State private var composerControl: ComposerControl?
    /// The last question read aloud, so each is read once.
    @State private var spokenQuestionId: String?
    /// Only rounds that arrive while the chat is open are read; one already waiting isn't read out of the blue.
    @State private var openedAt = Date()
    /// A spoken command that needs a yes/no ("Skip this question?").
    @State private var grillSuggestion: GrillMatcher.Intent?
    /// The question the suggestion is about, so a reload that briefly drops the current question doesn't clear it.
    @State private var suggestionFor: String?
    @State private var commandCheck: Task<Void, Never>?
    @Environment(Speaker.self) private var speaker
    @AppStorage(Prefs.grillReadAloud) private var grillReadAloud = true
    @AppStorage(Prefs.voiceHandsFree) private var handsFree = true
    @AppStorage(Prefs.voiceReadReplies) private var readReplies = true
    /// After a message goes out by voice: wait for the reply, read it, open the mic again.
    @State private var voiceLoop: Task<Void, Never>?
    /// The on-device model's first reply to what you just sent, shown (and read, in a voice turn) until the real one comes.
    @State private var quickAck: QuickAck?
    @State private var quickAckTask: Task<String?, Never>?
    /// "Send to <agent>?" for a message to the Orchestrator that looks meant for one agent.
    @State private var routeSuggestion: RouteSuggestion?
    /// A task picked in "Attach an issue", sent with the next message (`meta.task_id`).
    @State private var attached: HBTask?
    /// The message picked with Reply, quoted above the message box and sent with the next message (spec #572).
    @State private var replyingTo: MessageQuote?
    /// Photos and a video picked in the composer, uploading or up, until they go with a message.
    @State private var attachments: [AttachmentDraft] = []
    @State private var pickingTask = false
    /// "Resume a session" in the context menu: the agent's past Sessions.
    @State private var resuming = false
    /// The agent's "/" menu, read again each time a run ends (every run reports its list afresh).
    @State private var commands: [AgentCommand] = []
    /// The on-device model's replies to the agent's latest message, when it ends on a question.
    @State private var suggested: (messageId: Int64, replies: [String])?
    private let dictate: Bool
    /// The message to scroll to once the chat has loaded, instead of the bottom (a saved item's Open in chat).
    @State private var focus: Int64?
    @Environment(VoiceSession.self) private var voice

    /// `-fixture-voice ask`: the Send it? question shows at once, for screenshots.
    private static var fixtureAsks: Bool {
        #if DEBUG
        let a = ProcessInfo.processInfo.arguments
        if let i = a.firstIndex(of: "-fixture-voice"), i + 1 < a.count { return a[i + 1] == "ask" }
        #endif
        return false
    }

    init(agentId: String, draft: String? = nil, dictate: Bool = false, message: Int64? = nil) {
        self.agentId = agentId
        self.dictate = dictate
        _focus = State(initialValue: message)
        _draft = State(initialValue: draft ?? "")
        // Talk's tidied text, landed in the box: it gets the Send it? bar until it's sent or touched.
        _talkText = State(initialValue: VoicePresentation.shared.statusAgentId == agentId ? draft : nil)
    }

    /// Talk's text waiting in the box, while its Send it? bar shows.
    @State private var talkText: String?

    var body: some View {
        if let agent = store.agent(agentId) {
            chat(agent)
        } else {
            EmptyState(symbol: "bubble.left", title: "Agent not found", message: "It may have been deleted on the Mac.")
        }
    }

    /// Draws the chat down to message `id` and scrolls to it, for a reply's quote or a saved item's Open in chat.
    /// False when it isn't loaded.
    private func reveal(_ id: Int64, _ proxy: ScrollViewProxy) -> Bool {
        let all = ChatItem.build(store.conversation(agentId))
        let key = ChatItem.key(message: id)
        guard let i = all.firstIndex(where: { $0.id == key }) else { return false }
        keepTop = nil
        shown = max(shown, all.count - i)
        DispatchQueue.main.async { withAnimation { proxy.scrollTo(key, anchor: .top) } }
        return true
    }

    private func chat(_ agent: Agent) -> some View {
        let all = ChatItem.build(store.conversation(agentId))
        let items = Array(all.suffix(shown))
        let state = store.state(of: agent)
        return ScrollViewReader { proxy in
            ScrollView {
                // A plain stack, not a lazy one: a lazy list's estimated row heights never settle around rows that
                // differ a lot in height (plan cards, interview questions, mention replies, large text), so the
                // chat kept working and UI tests and VoiceOver never saw it idle. Only the latest `shown` items
                // are drawn, so a long history stays cheap.
                VStack(alignment: .leading, spacing: 10) {
                    if all.count > shown || store.historyHasMore[agentId] == true {
                        Button {
                            // Keep your place: the item at the top stays at the top once the earlier ones are in.
                            // Not a row of tool calls: one cut by the page boundary takes a new id once the
                            // earlier calls in its run arrive.
                            func reveal() {
                                keepTop = items.first { if case .tools = $0 { false } else { true } }?.id
                                shown += Self.page
                            }
                            if all.count > shown { reveal(); return }
                            Task {
                                loadingOlder = true
                                try? await store.loadOlder(agentId)
                                // Nothing new (or the fetch failed): no earlier page will be drawn to scroll back
                                // from, and a kept item left set would pull you up later, once new messages arrive.
                                if ChatItem.build(store.conversation(agentId)).count > all.count { reveal() }
                                loadingOlder = false
                            }
                        } label: {
                            if loadingOlder { ProgressView() } else { Text("Load earlier messages") }
                        }
                        .font(.footnote)
                        .frame(maxWidth: .infinity)
                    }
                    if loading && items.isEmpty {
                        ProgressView().frame(maxWidth: .infinity).padding(.top, 40)
                    } else if items.isEmpty {
                        EmptyState(symbol: "bubble.left", title: "No messages yet",
                                   message: "Say what you'd like \(agent.name) to do. It runs on your Mac and replies here.")
                            .padding(.top, 40)
                    }
                    ForEach(items) { item in
                        ChatItemView(item: item, agent: agent, currentQuestionId: currentQuestion?.id, command: grillCommand, clearWith: clearWith,
                                     reply: store.info?.supports(.reply) == true ? { replyingTo = MessageQuote(of: $0, agentName: agent.name) } : nil,
                                     copyToBox: { draft = QueueMark.copiedToBox(draft, $0) },
                                     jump: { _ = reveal($0, proxy) })
                            .id(item.id)
                    }
                    ForEach(store.pending(for: agentId)) { p in
                        PendingBubble(message: p).id(p.id)
                    }
                    if let a = shownQuickAck { QuickAckRow(text: a.text).id("quickAck") }
                    if state == .working {
                        RunIndicator(startedAt: agent.lastRunAt, status: agent.statusText).id("working")
                    } else if grill == nil, awaitingReply(agent), shownQuickAck == nil {
                        // Right after you send: an instant "got it" until the agent's run shows up.
                        WorkingIndicator(text: "Got it, thinking…").id("working")
                    } else if let s = grill, s.state == .preparing || s.waiting == true {
                        // The agent is writing the first round, or the next one after you sent yours.
                        WorkingIndicator(text: s.state == .preparing ? "Preparing questions…" : "Next round…").id("working")
                    } else if let run = RunTimer.lastFinished(store.conversation(agentId)) {
                        RunDoneLine(text: RunTimer.done(run.duration, endedAt: run.endedAt)).id("runDone")
                    }
                    ForEach(store.pendingApprovals(for: agentId)) { ApprovalCard(request: $0) }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, Theme.gutter)
                .padding(.vertical, 12)
            }
            // Open on the newest messages. On a cold launch the cached chat is there before the first layout, and a
            // scrollTo then lands nowhere (#625).
            .defaultScrollAnchor(.bottom)
            // Jump to the bottom once it has messages; new ones scroll it down below.
            .task(id: !items.isEmpty) {
                guard !items.isEmpty else { return }
                // Or to the saved message Open in chat came for, loading earlier pages until it's there.
                if let id = focus {
                    focus = nil
                    _ = await store.loadMessage(id, in: agentId)
                    if reveal(id, proxy) { return }
                }
                proxy.scrollTo("bottom", anchor: .bottom)
            }
            .scrollDismissesKeyboard(.interactively)
            // Scroll back to the kept item once the earlier page is in the view, not right after the tap: after a
            // server fetch a scroll queued then runs before the new page is laid out, and you land on its top (#630).
            .onChange(of: items.first?.id) {
                guard let keep = keepTop else { return }
                keepTop = nil
                DispatchQueue.main.async { proxy.scrollTo(keep, anchor: .top) }
            }
            .onChange(of: items.last?.id) { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
            .onChange(of: store.pending(for: agentId).count) { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
            // The questions card replaces the permission card: keep it in view.
            .onChange(of: store.pendingApprovals(for: agentId).map(\.id)) { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
            .onChange(of: quickAck) { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
        }
        .background(Theme.background)
        .safeAreaInset(edge: .top) {
            VStack(spacing: 6) {
                if let loadError, items.isEmpty {
                    Label(loadError.title, systemImage: "exclamationmark.triangle").font(.footnote).foregroundStyle(Theme.failure)
                }
                StaleBanner()
            }
            .padding(.horizontal, Theme.gutter)
        }
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 0) {
                ReadAloudBar()
                if let intent = grillSuggestion {
                    GrillSuggestionBar(intent: intent, yes: {
                        grillSuggestion = nil
                        composerControl = .cancelDictation
                        draft = ""
                        perform(intent)
                    }, no: { grillSuggestion = nil })
                    .padding(.horizontal, Theme.gutter)
                    .padding(.top, 6)
                    .background(.bar)
                }
                if grill != nil {
                    Button { showingInterview = true } label: {
                        Label("Open interview", systemImage: "arrow.up.left.and.arrow.down.right").frame(maxWidth: .infinity, minHeight: 32)
                    }
                    .buttonStyle(.bordered)
                    .padding(.horizontal, Theme.gutter)
                    .padding(.top, 6)
                    .background(.bar)
                    .accessibilityHint("Shows one question at a time, full screen")
                    .accessibilityIdentifier("openInterview")
                }
                RunWatchBar(agentId: agentId)
                if let s = suggested, s.messageId == askingMessage?.id, draft.isEmpty, !s.replies.isEmpty {
                    SuggestedReplies(replies: s.replies) { r in
                        VoiceLog.shared.add("ai: suggested reply picked")
                        draft = r
                    }
                    .padding(.horizontal, Theme.gutter)
                    .padding(.top, 2)
                    .background(.bar)
                    .accessibilityIdentifier("chatSuggestions")
                }
                if agent.loop != nil || !(agent.schedules ?? []).isEmpty {
                    LoopScheduleBar(agent: agent,
                                    stop: { runCommand(.loop("stop"), text: "") },
                                    cancel: { id in
                                        Task { do { try await store.cancelSchedule(agentId, id: id) } catch { actionError = APIError.from(error) } }
                                    })
                    .padding(.horizontal, Theme.gutter)
                    .padding(.top, 6)
                    .background(.bar)
                }
                if let r = routeSuggestion, let target = store.agent(r.agentId) {
                    RouteSuggestionBar(agentName: target.name, send: { sendRouted(r) }, dismiss: {
                        VoiceLog.shared.add("ai: “Send to \(target.name)?” dismissed")
                        routeSuggestion = nil
                    })
                    .padding(.horizontal, Theme.gutter)
                    .padding(.top, 6)
                    .background(.bar)
                }
                if let q = replyingTo {
                    HStack(spacing: 6) {
                        Image(systemName: "arrowshape.turn.up.left").foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text("Replying to \(q.authorLabel)").font(.caption.weight(.semibold)).foregroundStyle(Theme.accent)
                            Text(verbatim: q.excerpt).font(.footnote).foregroundStyle(.secondary).lineLimit(1)
                        }
                        Spacer(minLength: 0)
                        Button { replyingTo = nil } label: { Image(systemName: "xmark.circle.fill").frame(width: 44, height: 44) }
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("Remove the quote")
                    }
                    .padding(.horizontal, Theme.gutter)
                    .background(.bar)
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("Replying to \(q.authorLabel): \(q.excerpt)")
                    .accessibilityIdentifier("replyQuote")
                }
                if let t = attached {
                    HStack(spacing: 6) {
                        Image(systemName: "paperclip").foregroundStyle(.secondary)
                        Text(t.title).font(.footnote).lineLimit(1)
                        Spacer(minLength: 0)
                        Button { attached = nil } label: { Image(systemName: "xmark.circle.fill").frame(width: 44, height: 44) }
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("Remove the attached issue")
                    }
                    .padding(.horizontal, Theme.gutter)
                    .background(.bar)
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("Attached issue: \(t.title)")
                    .accessibilityIdentifier("attachedTask")
                }
                if let t = talkText, !t.isEmpty {
                    TalkConfirmBar(text: t, current: draft, note: VoicePresentation.shared.recipientNote, startAsking: Self.fixtureAsks, send: {
                        talkText = nil
                        draft = ""
                        VoicePresentation.shared.statusAgentId = nil
                        VoicePresentation.shared.recipientNote = nil
                        voice.sendWithStatus(t, to: agentId)
                    }, cancel: {
                        talkText = nil
                        draft = ""
                        VoicePresentation.shared.statusAgentId = nil
                        VoicePresentation.shared.recipientNote = nil
                    }, done: {
                        talkText = nil
                        VoicePresentation.shared.recipientNote = nil
                    })
                    .id(t)
                }
                ComposerView(text: $draft, recipient: agent.name, agentId: agentId, isOrchestrator: store.orchestratorId == agent.id,
                             commands: currentQuestion == nil ? commands : [],
                             control: $composerControl,
                             onDictation: currentQuestion == nil ? nil : { heardWhileGrilling($0) },
                             holdAutoSend: { grillSuggestion != nil },
                             dictateOnAppear: dictate,
                             startVoice: { Task { await voice.start(agentId: agentId) } },
                             // In an interview the chat's own dictation drives it, so the mic stays as it was.
                             tapOpensTalk: grill == nil,
                             onVoiceSent: { continueByVoice() },
                             onVoiceStop: stopByVoice,
                             attachTask: currentQuestion == nil ? { pickingTask = true } : nil,
                             videoAgentId: currentQuestion == nil ? agentId : nil, attachments: $attachments) { text, byVoice in
                    if attachments.isEmpty, let action = CommandQuery.action(for: text, commands: commands), currentQuestion == nil {
                        runCommand(action, text: text)
                    } else if !attachments.isEmpty {
                        // A message with photos or a video is always plain chat.
                        let meta = MessageQuote.meta(attached.map { .object(["task_id": .string($0.id)]) }, replyingTo: replyingTo)
                        let files = attachments.compactMap(\.uploaded)
                        attached = nil
                        replyingTo = nil
                        attachments = []
                        Task { await store.send(text, to: agentId, meta: meta, images: files) }
                    // While a round waits, a voice turn answers the question the voice is on (tagged so it can't land
                    // on another one) and the voice moves on; typed text is always chat, as on the web.
                    } else if store.composerAnswers(byVoice: byVoice, agentId: agentId) {
                        Task { do { try await store.answerCurrentGrill(text, agentId: agentId) } catch { actionError = APIError.from(error) } }
                    } else if VoicePresentation.shared.statusAgentId == agentId, attached == nil {
                        // A Talk dictation sent from here gets the full-screen status (Sent, Thinking, the reply).
                        VoicePresentation.shared.statusAgentId = nil
                        voice.sendWithStatus(text, to: agentId)
                    } else {
                        quickReply(to: text)
                        let meta = MessageQuote.meta(attached.map { .object(["task_id": .string($0.id)]) }, replyingTo: replyingTo)
                        attached = nil
                        replyingTo = nil
                        Task { await store.send(text, to: agentId, meta: meta) }
                    }
                }
            }
        }
        .navigationTitle(agent.name)
        .task(id: "\(agentId)-\(agent.running)") { commands = await store.commands(agentId) }
        // Undo on Talk's status screen: the text comes back to this message box (the chat stays the same view).
        .onChange(of: VoicePresentation.shared.returnedText, initial: true) { _, t in
            guard let t, VoicePresentation.shared.statusAgentId == agentId else { return }
            draft = t
            talkText = t
            VoicePresentation.shared.returnedText = nil
        }
        // Sent from Talk: the reply is waited for and read here, as for a message dictated in this chat.
        .onChange(of: VoicePresentation.shared.replyAfter, initial: true) { _, h in
            guard let h, h.agentId == agentId else { return }
            VoicePresentation.shared.replyAfter = nil
            continueByVoice(after: h.after)
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(agent.name).font(.headline)
                    let label = state.label(pending: store.pendingApprovals(for: agent.id))
                    Text(store.orchestratorId == agent.id ? "Orchestrator · \(label)" : "Direct · \(label)")
                        .font(.caption2)
                        .foregroundStyle(state.color)
                }
                .accessibilityElement(children: .combine)
            }
            ToolbarItem(placement: .topBarTrailing) { contextMenu(agent, running: state == .working || agent.running == true) }
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    NavigationLink(value: AgentRoute(id: agent.id)) { Label("Agent details", systemImage: "person.crop.circle") }
                    if let t = store.currentTask(for: agent.id) {
                        NavigationLink(value: TaskRoute(id: t.id)) { Label("Current issue", systemImage: "checklist") }
                    }
                    // Interviews start when the agent asks; there's no mode to switch on. While one runs it can be paused or ended here.
                    if let s = grill {
                        if s.state == .active || s.state == .paused {
                            Button { grillCommand(s.state == .paused ? .resume : .pause) } label: {
                                Label(s.state == .paused ? "Resume interview" : "Pause interview",
                                      systemImage: s.state == .paused ? "play.circle" : "pause.circle")
                            }
                        }
                        Button(role: .destructive) { confirmEndGrill = true } label: { Label("End interview", systemImage: "flag.checkered") }
                    }
                    if state == .working || agent.running == true {
                        Button(role: .destructive) { confirmStop = true } label: { Label("Stop current run", systemImage: "stop.circle") }
                    }
                } label: {
                    // Filled and hierarchical like the Create button: the outline version nearly vanished in light mode.
                    Image(systemName: "ellipsis.circle.fill")
                        .font(.title3)
                        .symbolRenderingMode(.hierarchical)
                }
                .accessibilityLabel("Conversation options")
                .confirmationDialog("Stop \(agent.name)'s current run?", isPresented: $confirmStop, titleVisibility: .visible) {
                    Button("Stop run", role: .destructive) {
                        Task { do { try await store.stopRun(agent.id) } catch { actionError = APIError.from(error) } }
                    }
                } message: {
                    Text("This ends the Claude process running now and drops messages waiting for it.")
                }
                .confirmationDialog("End the interview?", isPresented: $confirmEndGrill, titleVisibility: .visible) {
                    Button("End", role: .destructive) { grillCommand(.end) }
                } message: {
                    Text("\(agent.name) will sum up what you covered.")
                }
            }
        }
        .errorAlert($actionError)
        .task(id: agentId) {
            openedAt = Date()
            await store.refreshGrill(agentId)
            #if DEBUG
            // `-fixture-grill-heard "skip that one"`: as if that was just dictated (screenshots, UI tests).
            if let heard = FixtureAPI.launchText("fixture-grill-heard") {
                try? await Task.sleep(for: .seconds(1))
                draft = heard
                heardWhileGrilling(heard)
            }
            // `-fixture-read-aloud YES`: read the last reply as a hands-free conversation would (the Skip/Stop bar).
            if UserDefaults.standard.bool(forKey: "fixture-read-aloud") {
                // Wait for the conversation to load.
                for _ in 0..<20 where !store.conversation(agentId).contains(where: { $0.role == "assistant" && $0.kind == "text" }) {
                    try? await Task.sleep(for: .milliseconds(250))
                }
            }
            if UserDefaults.standard.bool(forKey: "fixture-read-aloud"),
               let m = store.conversation(agentId).last(where: { $0.role == "assistant" && $0.kind == "text" }) {
                speaker.holdFixture(text: m.displayContent, messageId: m.id, hasMore: VoiceText.shortRead(m.displayContent).rest != nil)
            }
            #endif
        }
        .onChange(of: currentQuestion?.id) { _, id in
            // A different question (or the interview ending) makes the suggestion moot; a reload doesn't.
            if let id, id != suggestionFor { grillSuggestion = nil }
            guard let id, id != spokenQuestionId, let q = currentQuestion else { return }
            let n = currentQuestionNumber ?? 0
            VoiceLog.shared.add("question \(n) is next")
            // A round already waiting when the chat opens isn't read out of the blue (it's on the cards), unless
            // the voice has already moved through it.
            let posted = store.grillRoundMessage(for: agentId)?.createdAt ?? .distantPast
            guard posted > openedAt.addingTimeInterval(-5) || store.grillVoiceAt[agentId] != nil else {
                return VoiceLog.shared.add("question \(n): not read, its round was already waiting when the chat opened")
            }
            spokenQuestionId = id
            readAloud(q)
        }
        .onChange(of: grill == nil) { _, closed in
            // However it ended (End, a spoken "end", a timeout, the Orchestrator), the mic goes off and stays off.
            if closed { interviewEnded() }
        }
        .onDisappear(perform: stopVoiceWork)
        .sheet(isPresented: $pickingTask) { TaskPicker { attached = $0 } }
        .fullScreenCover(isPresented: $showingInterview) { InterviewView(agent: agent) }
        .sheet(isPresented: $resuming) { ResumeSheet(agentId: agentId) }
        .task(id: askingMessage?.id) {
            guard let m = askingMessage, suggested?.messageId != m.id else { return }
            let replies = await OnDeviceFeatures.chatReplies(to: m.displayContent)
            guard !Task.isCancelled else { return }
            suggested = (m.id, replies)
        }
        .task(id: agentId) {
            loading = true
            do { try await store.loadConversation(agentId); loadError = nil } catch { loadError = APIError.from(error) }
            loading = false
        }
        .onChange(of: store.isLive) { _, live in
            // History may have moved on while disconnected.
            if live { Task { try? await store.loadConversation(agentId) } }
        }
    }
}

extension ChatView {
    fileprivate var grill: GrillSession? { store.openGrill(for: agentId) }

    /// The agent's latest message, when it's the last word in the chat and ends on a question (not during an
    /// interview, which has its own choices, nor while a message of yours is on its way).
    private var askingMessage: ChatMessage? {
        guard grill == nil, store.pending(for: agentId).isEmpty,
              let m = store.conversation(agentId).last(where: { $0.kind == "text" || $0.role == "user" }),
              m.role == "assistant", m.kind == "text", ChatReplies.asksSomething(m.displayContent) else { return nil }
        return m
    }

    /// You sent something in the last couple of minutes and nothing has come back yet.
    fileprivate func awaitingReply(_ agent: Agent) -> Bool {
        guard agent.status != "error",
              let last = store.conversation(agentId).last(where: { ["text", "error"].contains($0.kind) })
        else { return false }
        return last.role == "user" && last.fromAgentName == nil && last.taskRole == nil && Date().timeIntervalSince(last.createdAt) < 120
    }
    fileprivate var currentQuestion: GrillQuestion? { store.currentGrillQuestion(for: agentId) }
    fileprivate var currentQuestionNumber: Int? { currentQuestion.flatMap { grill?.number(of: $0.id) } }

    /// Reads a question in the chosen voice (its short `speak` text, without links or PR numbers, and its
    /// recommendation), then opens the mic when hands-free is on. With VoiceOver running, VoiceOver announces it instead.
    fileprivate func readAloud(_ q: GrillQuestion) {
        let n = grill?.number(of: q.id) ?? 1
        let text = q.spoken(number: n, of: grill?.questions?.count ?? 1, language: grill?.language)
        if UIAccessibility.isVoiceOverRunning {
            VoiceLog.shared.add("question \(n): announced by VoiceOver")
            UIAccessibility.post(notification: .announcement, argument: text)
            return
        }
        guard grillReadAloud else { return VoiceLog.shared.add("question \(n): not read, “Read questions aloud” is off") }
        // The Talk screen reads the next question itself; two readers would cut each other off.
        guard !voice.readsReplies else { return VoiceLog.shared.add("question \(n): left to the Talk screen") }
        Task {
            // A question that arrives while the mic is on (no words yet): mic off first, so the reader gets the audio.
            if draft.isEmpty {
                composerControl = .stopListening
                try? await Task.sleep(for: .milliseconds(150))
            }
            speaker.phrases = store.grillPhrases
            VoiceLog.shared.add("question \(n): reading")
            let ended = await speaker.speak(text, messageId: store.grillRoundMessage(for: agentId)?.id, interruptible: handsFree)
            VoiceLog.shared.add("question \(n): read \(ended.map { "until “\($0)”" } ?? "through")")
            switch ended {
            case .skip:
                grillCommand(.skip)
                return
            case .repeatIt:
                readAloud(q)
                return
            case .stop, .more, nil:
                // "Stop" only cuts the reading short: the question still waits for its answer.
                break
            }
            // Still the same question, and nothing typed meanwhile.
            if handsFree, currentQuestion?.id == q.id, draft.isEmpty { composerControl = .listen }
        }
    }

    /// Hands-free chat: once the reply to a dictated message is in, read it and listen again, so after
    /// the first tap on the mic nothing needs a tap. A grill question that comes back is read by `readAloud`.
    /// `after`: the last message before the one sent, when it went out before this chat opened (from Talk).
    fileprivate func continueByVoice(after: Int64? = nil) {
        let before = after ?? store.conversation(agentId).last?.id ?? 0
        let wasGrilling = grill != nil
        voiceLoop?.cancel()
        voiceLoop = Task {
            // The quick acknowledgement first, while the real reply is worked on.
            if after == nil, readReplies, !UIAccessibility.isVoiceOverRunning, let ack = await quickAckTask?.value, !Task.isCancelled {
                VoiceLog.shared.add("ai: reading the quick acknowledgement")
                await speaker.speak(ack)
            }
            VoiceLog.shared.add("reply: waiting for \(store.agent(agentId)?.name ?? "the agent")")
            guard let answer = await store.waitForReply(agentId: agentId, after: before, while: { !Task.isCancelled }) else {
                return VoiceLog.shared.add("reply: stopped waiting (left the chat or said stop)")
            }
            guard !Task.isCancelled else { return }
            VoiceLog.shared.add("reply arrived: \(VoiceLog.clip(answer))")
            // An interview question is read by `readAloud` when it arrives.
            guard currentQuestion == nil else { return VoiceLog.shared.add("reply: it's an interview question; read as one") }
            // That answer ended the interview: don't start talking or listening again.
            if wasGrilling, grill == nil { return VoiceLog.shared.add("reply: the interview ended; not read") }
            if UIAccessibility.isVoiceOverRunning {
                // VoiceOver reads it; opening the mic would talk over it.
                VoiceLog.shared.add("reply: announced by VoiceOver")
                UIAccessibility.post(notification: .announcement, argument: VoiceText.speakable(answer).text)
                return
            }
            if readReplies {
                let id = store.conversation(agentId).last { $0.role == "assistant" && $0.kind == "text" }?.id
                VoiceLog.shared.add("reply: reading")
                // "Stop" ends the conversation; "skip" goes straight to your turn.
                let ended = await speaker.readReply(answer, messageId: id, isActive: { !Task.isCancelled })
                VoiceLog.shared.add(speaker.lastFailure.map { "reply: not read: \($0)" } ?? "reply: read \(ended.map { "until “\($0)”" } ?? "through")")
                if ended == .stop { return }
            } else {
                VoiceLog.shared.add("reply: not read, “Read replies aloud” is off")
            }
            guard !Task.isCancelled, handsFree, draft.isEmpty, currentQuestion == nil else { return }
            composerControl = .listen
        }
    }

    fileprivate func interviewEnded() {
        VoiceLog.shared.add("interview ended: mic off, reading stopped")
        composerControl = .stopListening
        commandCheck?.cancel()
        voiceLoop?.cancel()
        grillSuggestion = nil
        if speaker.interruptible { speaker.stop() }
    }

    /// The quick acknowledgement still waiting for the real reply.
    fileprivate var shownQuickAck: QuickAck? {
        guard let a = quickAck, a.agentId == agentId else { return nil }
        let replied = store.conversation(agentId).contains { $0.id > a.after && $0.role == "assistant" && $0.kind == "text" }
        return replied || Date().timeIntervalSince(a.at) > 120 ? nil : a
    }

    /// On sending to the Orchestrator: the on-device model's acknowledgement and route guess, in parallel and
    /// within ~1.5 s each; neither sends anything.
    fileprivate func quickReply(to text: String) {
        let before = store.conversation(agentId).last?.id ?? 0
        let id = agentId
        quickAck = nil
        routeSuggestion = nil
        guard store.orchestratorId == agentId else { return }
        let task = Task { await OnDeviceFeatures.quickAck(for: text) }
        quickAckTask = task
        Task {
            guard let ack = await task.value else { return }
            VoiceLog.shared.add("ai: quick acknowledgement: \(ack)")
            quickAck = QuickAck(agentId: id, after: before, text: ack, at: Date())
        }
        #if canImport(FoundationModels)
        if #available(iOS 26.0, *) {
            Task {
                guard let target = await OnDeviceFeatures.routeGuess(for: text, agents: store.agents, orchestratorId: store.orchestratorId),
                      agentId == id else { return }
                VoiceLog.shared.add("ai: route guess: \(target.name); offering “Send to \(target.name)?”")
                routeSuggestion = RouteSuggestion(text: text, agentId: target.id)
            }
        }
        #endif
    }

    /// Tapped "Send to <agent>": the same message goes to that agent too. Nothing is sent without this tap.
    fileprivate func sendRouted(_ r: RouteSuggestion) {
        routeSuggestion = nil
        VoiceLog.shared.add("ai: sent to \(store.agent(r.agentId)?.name ?? r.agentId) by tap")
        Task { await store.send(r.text, to: r.agentId) }
    }

    fileprivate func stopVoiceWork() {
        commandCheck?.cancel()
        voiceLoop?.cancel()
    }

    /// "Stop" ends the hands-free chat; during an interview the grill commands handle it instead.
    fileprivate var stopByVoice: (() -> Void)? {
        guard currentQuestion == nil else { return nil }
        return { [voiceLoop, speaker] in
            voiceLoop?.cancel()
            speaker.stop()
        }
    }

    /// Dictation while a question waits: once the speaker pauses, a command runs (or asks first);
    /// anything else stays in the field as the answer.
    fileprivate func heardWhileGrilling(_ text: String) {
        commandCheck?.cancel()
        commandCheck = Task {
            try? await Task.sleep(for: .seconds(1.2))
            guard !Task.isCancelled else { return }
            let question = currentQuestion.map { $0.speak ?? $0.text } ?? ""
            let m = await GrillUnderstanding.read(text, question: question, phrases: store.grillPhrases,
                                                  model: OnDeviceFeatures.grillLabel)
            // More was said while the model thought: that text gets its own check.
            guard !Task.isCancelled else { return }
            guard let m else {
                // An early "next…" grew into an answer: drop its question, or it holds the auto-send.
                grillSuggestion = nil
                return
            }
            if m.confidence == .sure, m.intent != .command(.end) {
                composerControl = .cancelDictation
                perform(m.intent)
            } else {
                // Unsure, or "end": ask. Saying it again or tapping Yes runs it; No keeps the words as the answer.
                grillSuggestion = m.intent
                suggestionFor = currentQuestion?.id
            }
        }
    }

    fileprivate func perform(_ intent: GrillMatcher.Intent) {
        switch intent {
        case .repeatQuestion:
            if let q = currentQuestion { readAloud(q) }
        case .command(let c):
            grillCommand(c)
        }
    }

    /// The context size: amber from 80k, and a tap offers Compact, Clear and Resume for the Session, like the web.
    /// With no Session (as after Clear) it reads "no session" and offers only Resume.
    fileprivate func contextMenu(_ agent: Agent, running: Bool) -> some View {
        // Older servers don't send the Session id; a known size still means there is one.
        let session = agent.sessionId != nil || agent.contextTokens != nil
        return Menu {
            if running { Text("Stop the agent first.") }
            if session {
                Button { sessionAction { try await store.compact(agentId) } } label: {
                    Label("Compact", systemImage: "arrow.down.right.and.arrow.up.left")
                    Text("Summarize the conversation so later runs carry less.")
                }
                .disabled(running)
                .accessibilityIdentifier("contextCompact")
                Button { sessionAction { try await store.resetSession(agentId) } } label: {
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
            ContextChip(tokens: agent.contextTokens, session: session)
        }
        .accessibilityIdentifier("contextMenu")
    }

    fileprivate func sessionAction(_ action: @escaping () async throws -> Void) {
        Task { do { try await action() } catch { actionError = APIError.from(error) } }
    }

    /// The end-of-planning card's Clear context: a fresh session, with the next message waiting in the composer, unsent.
    fileprivate func clearWith(_ next: String) async {
        do {
            try await store.resetSession(agentId)
            draft = next
        } catch { actionError = APIError.from(error) }
    }

    /// A command typed or picked from the "/" menu, sent: the composer is emptied once it ran, kept if it failed.
    /// `text` is what was sent: the composer has already emptied itself by then.
    fileprivate func runCommand(_ action: CommandQuery.Action, text: String) {
        draft = ""
        if action == .resume { return resuming = true }
        Task {
            do { try await store.run(action, agentId: agentId) } catch {
                draft = text
                actionError = APIError.from(error)
            }
        }
    }

    fileprivate func grillCommand(_ c: GrillCommand) {
        if c == .end { interviewEnded() }
        Task {
            do { try await store.sendGrillCommand(c, agentId: agentId) } catch { actionError = APIError.from(error) }
        }
    }
}

/// What the chat shows: user and agent text, errors, approvals, and each run's
/// tool calls folded into one collapsible row.
enum ChatItem: Identifiable, Hashable {
    case message(ChatMessage)
    case tools(id: Int64, [ChatMessage])
    case divider(id: Int64, text: String, date: Date)

    var id: String {
        switch self {
        case .message(let m): Self.key(message: m.id)
        case .tools(let id, _): "t\(id)"
        case .divider(let id, _, _): "d\(id)"
        }
    }

    static func key(message id: Int64) -> String { "m\(id)" }

    static func build(_ messages: [ChatMessage]) -> [ChatItem] {
        var out: [ChatItem] = []
        var tools: [ChatMessage] = []
        func flush() {
            if let first = tools.first { out.append(.tools(id: first.id, tools)) }
            tools.removeAll()
        }
        for m in messages {
            switch m.kind {
            case "tool_use" where m.toolName == "ToolSearch":
                // Claude looking up which tools it has: bookkeeping, not work (the web chat hides it too).
                continue
            case "tool_use":
                tools.append(m)
            case "tool_result", "thinking", "run_start", "run_end", "permission":
                // Results belong to their tool call; approvals show as live cards; run markers aren't content.
                continue
            case "clear":
                flush()
                out.append(.divider(id: m.id, text: "Context cleared", date: m.createdAt))
            case "compact":
                flush()
                out.append(.divider(id: m.id, text: "Conversation compacted", date: m.createdAt))
            case "resume":
                // From Resume a session (here or the web's /resume): the log goes on, nothing reloads.
                flush()
                out.append(.divider(id: m.id, text: resumeText(m), date: m.createdAt))
            default:
                flush()
                out.append(.message(m))
            }
        }
        flush()
        return out
    }

    /// "Resumed the Session from Oct 20, 2026", the start date in local time as the web's Resume list shows it; the
    /// stored text names the UTC day (#103).
    static func resumeText(_ m: ChatMessage, timeZone: TimeZone = .current, locale: Locale = .current) -> String {
        guard let s = m.meta?["started_at"]?.stringValue, let start = JSONCoding.parseDate(s) else {
            return m.content.trimmingCharacters(in: CharacterSet(charactersIn: "."))
        }
        let day = start.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted, locale: locale, timeZone: timeZone))
        return "Resumed the Session from \(day)"
    }
}

private struct ChatItemView: View {
    let item: ChatItem
    let agent: Agent
    var currentQuestionId: String?
    var command: (GrillCommand) -> Void = { _ in }
    var clearWith: (String) async -> Void = { _ in }
    /// Reply to a message (nil on a server without replies), and jump to a quoted one.
    var reply: ((ChatMessage) -> Void)?
    /// Copy to box on a cancelled message: its text goes into the message box, after any draft.
    var copyToBox: (ChatMessage) -> Void = { _ in }
    var jump: (Int64) -> Void = { _ in }

    var body: some View {
        switch item {
        case .message(let m) where m.taskRole != nil: TaskMessageRow(message: m)
        case .message(let m) where m.grill?.role == .round:
            GrillRoundCard(message: m, agent: agent, currentQuestionId: currentQuestionId, command: command)
        case .message(let m) where m.grill?.role == .status: GrillStatusRow(message: m)
        case .message(let m) where m.grill?.role == .summary:
            VStack(alignment: .leading, spacing: 4) {
                Label(m.grill?.final == true ? "Interview summary" : "Summary so far", systemImage: "text.alignleft")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Theme.accent)
                MessageBubble(message: m, agent: agent)
                if let card = m.grill?.endCard { EndCardView(message: m, card: card, clearWith: clearWith) }
            }
        case .message(let m): MessageBubble(message: m, agent: agent, reply: m.isQuotable && !m.isCancelled ? reply : nil, copyToBox: copyToBox, jump: jump)
        case .tools(_, let calls): ToolGroup(calls: calls)
        case .divider(_, let text, let date):
            HStack {
                VStack { Divider() }
                Text("\(text) · \(date.formatted(date: .abbreviated, time: .shortened))").font(.caption2).foregroundStyle(.secondary).fixedSize()
                VStack { Divider() }
            }
            .padding(.vertical, 4)
        }
    }
}

private struct MessageBubble: View {
    @Environment(LoftStore.self) private var store
    @Environment(Speaker.self) private var speaker
    let message: ChatMessage
    let agent: Agent
    var reply: ((ChatMessage) -> Void)?
    var copyToBox: (ChatMessage) -> Void = { _ in }
    var jump: (Int64) -> Void = { _ in }
    @State private var saving = false
    /// The saved item's title, shown under the message for a moment once Save is done.
    @State private var savedAs: String?

    private var isUser: Bool { message.role == "user" && message.fromAgentName == nil }
    private var canSave: Bool { store.info?.supports(.saved) == true && Saved.canSave(message) }
    @State private var cancelling = false
    @State private var cancelError: String?

    private var mark: QueueMark? { QueueMark(message, in: agent) }

    /// Takes the message back out of the agent's queue; a run that already took it says so under the message.
    private func cancelQueued() {
        cancelling = true
        cancelError = nil
        Task {
            do {
                if try await !store.cancelQueued(agentId: agent.id, messageId: message.id) { cancelError = QueueMark.tooLateText }
            } catch {
                cancelError = APIError.from(error).title
            }
            cancelling = false
        }
    }

    private var footerTime: some View {
        Text(isUser ? message.createdAt.formatted(date: .omitted, time: .shortened)
             : "\(message.fromAgentName == nil && message.role != "user" ? agent.name + " · " : "")\(message.createdAt.formatted(date: .omitted, time: .shortened))")
            .foregroundStyle(.tertiary)
    }

    @ViewBuilder private var footerActions: some View {
        if !isUser && message.role == "assistant" { SpeakButton(message: message) }
    }

    /// The footer's queue status (#668): the queued mark with Cancel, or the cancelled mark with Copy to box.
    @ViewBuilder private var queueStatus: some View {
        switch mark {
        case .queued?:
            // In the wrapped footer the mark wraps rather than truncate at large text sizes.
            Label(QueueMark.queuedText, systemImage: "hourglass").foregroundStyle(Theme.attentionInk)
                .fixedSize(horizontal: false, vertical: true)
            Button(action: cancelQueued) {
                Label("Cancel", systemImage: "xmark").labelStyle(.titleAndIcon)
            }
            .buttonStyle(.borderless)
            .tint(Theme.attentionInk)
            .disabled(cancelling)
            .frame(minHeight: 44)
            .accessibilityLabel(message.fromAgentName.map { "Cancel this queued message from \($0)" } ?? "Cancel this queued message")
            .accessibilityIdentifier("cancelQueued")
        case .cancelled?:
            Text(QueueMark.cancelledText).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Button { copyToBox(message) } label: {
                Label("Copy to box", systemImage: "doc.on.doc").labelStyle(.titleAndIcon)
            }
            .buttonStyle(.borderless)
            .frame(minHeight: 44)
            .accessibilityIdentifier("copyToBox")
        case nil:
            EmptyView()
        }
    }

    var body: some View {
        switch message.kind {
        case "error":
            Label {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Run failed").font(.footnote.weight(.semibold))
                    Text(message.content.split(separator: "\n").prefix(3).joined(separator: "\n")).font(.footnote)
                }
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
            }
            .foregroundStyle(Theme.failure)
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.failure.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
        case "notice":
            if let id = message.meta?["plan_task_id"]?.stringValue, store.info?.supports(.plans) == true {
                PlanCard(taskId: id, fallback: message.content)
            } else {
                Text(message.content).font(.footnote).foregroundStyle(.secondary).frame(maxWidth: .infinity)
            }
        default:
            let photos = message.photos.compactMap { store.uploadURL(agentId: message.agentId, file: $0) }
            if !photos.isEmpty {
                // Two a row, an odd one on the message's side.
                VStack(alignment: isUser ? .trailing : .leading, spacing: 6) {
                    ForEach(Array(stride(from: 0, to: photos.count, by: 2)), id: \.self) { i in
                        HStack(spacing: 6) { ForEach(photos[i..<min(i + 2, photos.count)], id: \.self) { ChatPhoto(url: $0) } }
                    }
                }
                .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
            }
            ForEach(message.videos, id: \.self) { f in
                if let url = store.uploadURL(agentId: message.agentId, file: f) {
                    ChatVideo(url: url).frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
                }
            }
            HStack(alignment: .bottom) {
                if isUser { Spacer(minLength: 48) }
                VStack(alignment: isUser ? .trailing : .leading, spacing: 4) {
                    if message.isMentionReply, let from = message.fromAgentName {
                        // Another agent answering your @-mention, in this chat: its sticker, not this chat's agent.
                        HStack(spacing: 6) {
                            AgentAvatar(name: from, id: message.fromAgentId ?? from, size: 20)
                            Text("\(from) · answering your mention").font(.caption2.weight(.semibold)).foregroundStyle(Theme.accent)
                        }
                        .accessibilityElement(children: .combine)
                    } else if let from = message.fromAgentName {
                        HStack(spacing: 6) {
                            Label("From \(from)", systemImage: "arrow.turn.down.right").font(.caption2.weight(.semibold)).foregroundStyle(Theme.accent)
                            if message.isFYI {
                                // Delivered without waking the agent: it reads it at the start of its next run.
                                Text("FYI")
                                    .font(.caption2.weight(.bold))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 1)
                                    .background(Theme.card, in: Capsule())
                                    .accessibilityIdentifier("fyiMark")
                            }
                        }
                    }
                    let tasks = message.taskRefs.compactMap { store.task($0) }
                    if !tasks.isEmpty {
                        ViewThatFits(in: .horizontal) {
                            HStack(spacing: 6) { ForEach(tasks) { TaskChip(task: $0) } }
                            VStack(alignment: isUser ? .trailing : .leading, spacing: 4) { ForEach(tasks) { TaskChip(task: $0) } }
                        }
                        .accessibilityIdentifier("taskChips")
                    }
                    if !message.tags.isEmpty {
                        ViewThatFits(in: .horizontal) {
                            HStack(spacing: 6) { ForEach(message.tags) { TagChip(tag: $0) } }
                            VStack(alignment: isUser ? .trailing : .leading, spacing: 4) { ForEach(message.tags) { TagChip(tag: $0) } }
                        }
                        .accessibilityIdentifier("tagChips")
                    }
                    if let q = message.replyQuote {
                        QuoteBlock(quote: q) { jump(q.id) }
                    }
                    if message.videos.isEmpty && message.photos.isEmpty || !message.displayContent.isEmpty {
                        CollapsibleText(text: message.displayContent, linkColor: isUser ? Theme.onAccent : nil)
                            .font(.body)
                            .strikethrough(message.isCancelled)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .foregroundStyle(isUser ? Theme.onAccent : .primary)
                            .background(isUser ? AnyShapeStyle(Theme.accent.gradient) : AnyShapeStyle(Theme.card),
                                        in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                            .opacity(message.isCancelled ? 0.5 : 1)
                            .contentShape(.contextMenuPreview, RoundedRectangle(cornerRadius: 18, style: .continuous))
                            .contextMenu {
                                if let reply {
                                    Button { reply(message) } label: { Label("Reply", systemImage: "arrowshape.turn.up.left") }
                                }
                                // The long press opens this menu instead of text selection: keep Copy in it.
                                Button { UIPasteboard.general.string = message.displayContent } label: { Label("Copy", systemImage: "doc.on.doc") }
                                if canSave {
                                    Button { saving = true } label: { Label("Save", systemImage: "bookmark") }
                                }
                            }
                            .sheet(isPresented: $saving) { SaveMessageSheet(message: message) { savedAs = $0.title } }
                    }
                    // One row: yours (right) the queue status, then the time; the others' (left) the time, then the status.
                    // On a narrow phone it wraps, with the status above and the time and actions on the last line (#666).
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 6) {
                            if isUser, mark != nil { queueStatus; Divider().frame(height: 12) }
                            footerTime
                            if !isUser, mark != nil { Divider().frame(height: 12); queueStatus }
                            footerActions
                        }
                        VStack(alignment: isUser ? .trailing : .leading, spacing: 0) {
                            HStack(spacing: 6) { queueStatus }
                            HStack(spacing: 6) { footerTime; footerActions }
                        }
                    }
                    .font(.caption2)
                    if let cancelError {
                        Text(cancelError).font(.caption2).foregroundStyle(Theme.failure).accessibilityIdentifier("cancelError")
                    }
                    if let savedAs {
                        Label("Saved “\(savedAs)”", systemImage: "bookmark.fill")
                            .font(.caption2)
                            .foregroundStyle(Theme.accent)
                            .transition(.opacity)
                    }
                    if !isUser && message.role == "assistant" { UnreadAloudNote(message: message) }
                    if isUser, !message.mentionedNames.isEmpty {
                        Label("\(message.mentionsOnly ? "Sent to" : "Also sent to") \(ListFormatter.localizedString(byJoining: message.mentionedNames))",
                              systemImage: "arrow.turn.up.right")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("mentionNote")
                    }
                }
                if !isUser { Spacer(minLength: 32) }
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel("\(isUser ? "You" : message.fromAgentName ?? agent.name)\(message.isFYI ? ", FYI" : ""): \(message.displayContent)")
            .accessibilityValue(mark == .queued ? QueueMark.queuedText : mark == .cancelled ? QueueMark.cancelledText : "")
            .accessibilityActions {
                // Only in its own state: Cancel while queued, Copy to box once cancelled.
                if mark == .queued { Button("Cancel this queued message", action: cancelQueued) }
                if mark == .cancelled { Button("Copy to box") { copyToBox(message) } }
            }
            .accessibilityAction(named: "Read aloud") { if !isUser { speaker.speak(message) } }
            .accessibilityAction(named: "Reply") { reply?(message) }
            .accessibilityAction(named: "Save") { if canSave { saving = true } }
            // Save's confirmation, as the web's toast: under the message for a moment, and spoken.
            .task(id: savedAs) {
                guard let t = savedAs else { return }
                AccessibilityNotification.Announcement("Saved “\(t)”").post()
                try? await Task.sleep(for: .seconds(3))
                withAnimation { savedAs = nil }
            }
            .accessibilityAction(named: "Go to the quoted message") { if let q = message.replyQuote { jump(q.id) } }
        }
    }
}

/// The message a reply quotes, above its bubble: who wrote it and a line of it, as plain text. A tap jumps to it.
private struct QuoteBlock: View {
    let quote: MessageQuote
    let jump: () -> Void

    var body: some View {
        Button(action: jump) {
            HStack(spacing: 8) {
                RoundedRectangle(cornerRadius: 2).fill(Theme.accent).frame(width: 3)
                VStack(alignment: .leading, spacing: 1) {
                    Text(quote.authorLabel).font(.caption.weight(.semibold)).foregroundStyle(Theme.accent)
                    Text(verbatim: quote.excerpt).font(.footnote).foregroundStyle(.secondary).lineLimit(2).multilineTextAlignment(.leading)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("messageQuote")
    }
}

/// While a hands-free conversation reads something aloud: big Skip and Stop buttons, for when you can't speak.
struct ReadAloudBar: View {
    @Environment(Speaker.self) private var speaker
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        if speaker.isSpeaking && speaker.interruptible {
            let layout = typeSize.isAccessibilitySize ? AnyLayout(VStackLayout(spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))
            layout {
                Label("Reading aloud", systemImage: "speaker.wave.2.fill")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityHidden(true)
                if speaker.hasMore {
                    Button { speaker.interrupt(.more) } label: {
                        Label("More", systemImage: "text.append").frame(minWidth: 72, minHeight: 36)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityHint("Reads the rest of the reply")
                    .accessibilityIdentifier("readAloudMore")
                }
                Button { speaker.interrupt(.skip) } label: {
                    Label("Skip", systemImage: "forward.fill").frame(minWidth: 72, minHeight: 36)
                }
                .buttonStyle(.bordered)
                .accessibilityHint("Stops reading and moves on")
                .accessibilityIdentifier("readAloudSkip")
                Button { speaker.interrupt(.stop) } label: {
                    Label("Stop", systemImage: "stop.fill").frame(minWidth: 72, minHeight: 36)
                }
                .buttonStyle(.accent)
                .accessibilityHint("Stops reading")
                .accessibilityIdentifier("readAloudStop")
            }
            .controlSize(.large)
            .padding(.horizontal, Theme.gutter)
            .padding(.vertical, 8)
            .background(.bar)
            .accessibilityIdentifier("readAloudBar")
        }
    }
}

/// "Couldn't read aloud · Retry" under a reply that should have been read but wasn't.
private struct UnreadAloudNote: View {
    @Environment(Speaker.self) private var speaker
    let message: ChatMessage

    var body: some View {
        if speaker.unreadMessageId == message.id && !speaker.isSpeaking {
            Button {
                VoiceLog.shared.add("reply: Retry reading tapped")
                speaker.speak(message)
            } label: {
                Label("Couldn't read aloud · Retry", systemImage: "speaker.slash")
                    .font(.caption)
                    .frame(minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(Theme.attention)
            .accessibilityLabel("Couldn't read aloud. Retry")
            .accessibilityIdentifier("retryReadAloud")
        }
    }
}

struct QuickAck: Equatable {
    var agentId: String
    /// The last message before yours: the next reply after it replaces this.
    var after: Int64
    var text: String
    var at: Date
}

struct RouteSuggestion: Equatable {
    var text: String
    var agentId: String
}

/// The on-device model's one-line first reply, styled apart from real replies until one arrives.
private struct QuickAckRow: View {
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(text).font(.subheadline)
            Label("Quick reply from this iPhone · the real one is on its way", systemImage: "sparkles")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Theme.card.opacity(0.6), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay { RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Theme.accent.opacity(0.35), style: StrokeStyle(lineWidth: 1, dash: [4, 3])) }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Quick reply: \(text)")
        .accessibilityIdentifier("quickAck")
    }
}

/// The agent's running `/loop` with a Stop, and its `/schedule` entries each with a Cancel, above the composer.
private struct LoopScheduleBar: View {
    let agent: Agent
    let stop: () -> Void
    let cancel: (String) -> Void

    private func every(_ minutes: Int) -> String {
        minutes % 1440 == 0 ? "\(minutes / 1440)d" : minutes % 60 == 0 ? "\(minutes / 60)h" : "\(minutes)m"
    }

    // At the largest text sizes the rows would fill the screen: the bar stops growing at this height and scrolls.
    @ScaledMetric(relativeTo: .body) private var maxHeight: CGFloat = 130
    @State private var contentHeight: CGFloat = 0

    var body: some View {
        ScrollView {
            rows
                .background(GeometryReader { g in Color.clear.preference(key: BarHeightKey.self, value: g.size.height) })
        }
        .onPreferenceChange(BarHeightKey.self) { contentHeight = $0 }
        .frame(height: contentHeight == 0 ? nil : min(contentHeight, min(maxHeight, 220)))
        .scrollBounceBehavior(.basedOnSize)
        .padding(10)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("loopSchedules")
    }

    private var rows: some View {
        VStack(spacing: 8) {
            if let l = agent.loop {
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(l.prompt).font(.footnote).lineLimit(1)
                        Text("Looping every \(every(l.intervalMinutes)) · next \(AgentSchedule.nextLabel(l.nextAt))")
                            .font(.caption2).foregroundStyle(.secondary).lineLimit(1).minimumScaleFactor(0.8)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Button("Stop", action: stop).buttonStyle(.bordered).frame(minHeight: 44).fixedSize().accessibilityIdentifier("loopStop")
                }
            }
            ForEach(agent.schedules ?? []) { s in
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(s.prompt).font(.footnote).lineLimit(1)
                        Text(s.rowLabel())
                            .font(.caption2).foregroundStyle(.secondary).lineLimit(1).minimumScaleFactor(0.8)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Button("Cancel") { cancel(s.id) }.buttonStyle(.bordered).frame(minHeight: 44).fixedSize().accessibilityIdentifier("scheduleCancel")
                }
            }
        }
    }
}

private struct BarHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// "Send to Web Dev?" above the composer. A tap sends the message there; nothing goes without it.
private struct RouteSuggestionBar: View {
    let agentName: String
    let send: () -> Void
    let dismiss: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Label("Send to \(agentName) too?", systemImage: "arrow.turn.up.right")
                .font(.footnote)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("Send", action: send)
                .buttonStyle(.accent)
                .accessibilityIdentifier("routeSend")
            Button(action: dismiss) { Image(systemName: "xmark") }
                .buttonStyle(.bordered)
                .accessibilityLabel("Not now")
                .accessibilityIdentifier("routeDismiss")
        }
        .controlSize(.small)
        .padding(10)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("routeSuggestion")
    }
}

/// Reads an agent's message aloud, or stops it.
private struct SpeakButton: View {
    @Environment(Speaker.self) private var speaker
    let message: ChatMessage
    private var playing: Bool { speaker.isSpeaking && speaker.messageId == message.id }

    var body: some View {
        Button {
            playing ? speaker.stop() : speaker.speak(message)
        } label: {
            Image(systemName: playing ? "stop.circle.fill" : "speaker.wave.2")
                .font(.caption)
                .foregroundStyle(playing ? Theme.accent : .secondary)
                .frame(minWidth: 44, minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(playing ? "Stop reading" : "Read aloud")
        .accessibilityIdentifier("speakButton")
    }
}

/// A run's tool calls, collapsed to one line. Raw inputs and outputs are never shown.
private struct ToolGroup: View {
    let calls: [ChatMessage]
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.snappy) { expanded.toggle() }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "wrench.and.screwdriver")
                    Text(calls.count == 1 ? (calls[0].content.isEmpty ? (calls[0].toolName ?? "Used a tool") : calls[0].content)
                         : "Used \(calls.count) tools")
                        .lineLimit(1)
                    Image(systemName: "chevron.right").rotationEffect(.degrees(expanded ? 90 : 0)).font(.caption2)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            if expanded {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(calls) { c in
                        Text("• " + (c.content.isEmpty ? (c.toolName ?? "tool") : c.content))
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                }
                .padding(.leading, 20)
            }
        }
        .padding(.vertical, 2)
    }
}

private struct PendingBubble: View {
    @Environment(LoftStore.self) private var store
    let message: PendingMessage

    var body: some View {
        HStack {
            Spacer(minLength: 48)
            VStack(alignment: .trailing, spacing: 4) {
                if let files = message.images, !files.isEmpty {
                    let photos = files.filter { !VideoAttachment.isVideo($0) }.count
                    if photos < files.count {
                        Label("Video", systemImage: "video.fill").font(.caption).foregroundStyle(.secondary)
                    }
                    if photos > 0 {
                        Label(photos == 1 ? "Photo" : "\(photos) photos", systemImage: "photo").font(.caption).foregroundStyle(.secondary)
                    }
                }
                if !message.text.isEmpty {
                    Text(message.text)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .foregroundStyle(Theme.onAccent)
                        .background(Theme.accent.opacity(message.isFailed ? 0.35 : 0.6), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                }
                if let failure = message.failure {
                    Label("Not sent: \(failure)", systemImage: "exclamationmark.circle")
                        .font(.caption)
                        .foregroundStyle(Theme.failure)
                        .multilineTextAlignment(.trailing)
                    // Real buttons with a 44pt hit area; they were bare caption links about 14pt tall.
                    HStack(spacing: 8) {
                        Button(role: .destructive) { store.discard(message) } label: {
                            Label("Delete", systemImage: "trash").padding(.vertical, 6)
                        }
                        .accessibilityIdentifier("discardSend")
                        Button { Task { await store.retry(message) } } label: {
                            Label("Retry", systemImage: "arrow.clockwise").padding(.vertical, 6)
                        }
                        .buttonStyle(.accent)
                        .disabled(!store.canWrite)
                        .accessibilityIdentifier("retrySend")
                    }
                    .buttonStyle(.bordered)
                    .buttonBorderShape(.capsule)
                    .font(.subheadline.weight(.medium))
                } else {
                    Label("Sending…", systemImage: "clock").font(.caption2).foregroundStyle(.tertiary)
                }
            }
        }
    }
}

/// The chat's run line while the agent works: an animated ✻, the time since the run's start ticking every second,
/// and the status text. Like the web's RunIndicator.
private struct RunIndicator: View {
    let startedAt: Date?
    let status: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Static in UI tests, like WorkingIndicator: a repeating animation keeps the screen from ever counting as idle.
    private static let uiTesting = ProcessInfo.processInfo.arguments.contains("-ui-testing") || TestMode.noAnimations

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { ctx in
            let elapsed = RunTimer.elapsed(since: startedAt, now: ctx.date)
            let text = status?.isEmpty == false ? "\(elapsed) · \(status!)" : elapsed
            HStack(spacing: 8) {
                star
                Text(text).monospacedDigit().lineLimit(2)
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
            .padding(.vertical, 6)
            .padding(.leading, 4)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Working: \(text)")
            .accessibilityIdentifier("workingIndicator")
        }
    }

    @ViewBuilder private var star: some View {
        let glyph = Text("✻").font(.body).foregroundStyle(Theme.accent)
        if reduceMotion || Self.uiTesting {
            glyph
        } else {
            // ✻ looks the same turned 180°, so the loop's jump back to 0° doesn't show.
            glyph.phaseAnimator([0.0, 90.0, 180.0]) { view, angle in
                view.rotationEffect(.degrees(angle))
                    .scaleEffect(angle == 90 ? 0.8 : 1)
                    .opacity(angle == 90 ? 0.55 : 1)
            } animation: { angle in
                angle == 0 ? nil : .easeInOut(duration: 1.2)
            }
        }
    }
}

/// `✻ Done in 2m 23s · Thu 16:23`, after the run until the next one.
private struct RunDoneLine: View {
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            Text("✻").font(.body).foregroundStyle(Theme.accent)
            Text(text).monospacedDigit().foregroundStyle(.tertiary)
        }
        .font(.footnote)
        .padding(.vertical, 6)
        .padding(.leading, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(text)
        .accessibilityIdentifier("runDone")
    }
}

private struct WorkingIndicator: View {
    let text: String
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// UI tests wait for animations to finish, and this one repeats while the agent works (always, in the fixtures):
    /// the screen never counts as idle and the test times out. Static there, as with Reduce Motion.
    private static let uiTesting = ProcessInfo.processInfo.arguments.contains("-ui-testing") || TestMode.noAnimations

    var body: some View {
        HStack(spacing: 8) {
            if reduceMotion || Self.uiTesting {
                Image(systemName: "ellipsis")
            } else {
                Image(systemName: "ellipsis").symbolEffect(.variableColor.iterative, options: .repeating)
            }
            Text(text).lineLimit(2)
        }
        .font(.footnote)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Theme.card, in: Capsule())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Working: \(text)")
        .accessibilityIdentifier("workingIndicator")
    }
}
