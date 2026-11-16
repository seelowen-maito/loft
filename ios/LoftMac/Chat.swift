import AppKit
import LoftKit
import SwiftUI
import UniformTypeIdentifiers

/// An agent's chat: its messages as they arrive, and the composer.
struct ChatView: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    let agent: Agent
    /// The agent's settings inspector, toggled from the header.
    @Binding var showsSettings: Bool
    @State private var loadError: APIError?
    @State private var draft = ""
    @State private var talking = false
    /// The text came from Talk, so the reply is read aloud once it's sent.
    @State private var draftByVoice = false
    /// The on-device model's replies to the agent's latest message, when it ends on a question.
    @State private var suggested: (messageId: Int64, replies: [String])?
    /// "Send to …?": the model thinks the message just sent to the Lead was meant for another agent.
    @State private var route: (text: String, agentId: String)?
    /// "Got it, looking into …": the on-device model's first reply to a message to the Orchestrator, shown until the
    /// real reply comes after message `after` (or for two minutes at most).
    @State private var quickAck: (agentId: String, after: Int64, text: String, at: Date)?
    /// The message picked with Reply, quoted above the message box and sent with the next message (spec #572).
    @State private var replyingTo: MessageQuote?

    private var shownQuickAck: String? {
        guard let a = quickAck, a.agentId == agent.id, Date().timeIntervalSince(a.at) < 120,
              !store.conversation(agent.id).contains(where: { $0.id > a.after && $0.role == "assistant" && $0.kind == "text" }) else { return nil }
        return a.text
    }

    /// The agent's latest message, when it's the last word in the chat and ends on a question.
    private var askingMessage: ChatMessage? {
        guard store.pending(for: agent.id).isEmpty,
              let m = store.conversation(agent.id).last(where: { $0.kind == "text" || $0.role == "user" }),
              m.role == "assistant", m.kind == "text", ChatReplies.asksSomething(m.displayContent) else { return nil }
        return m
    }

    /// What the chat shows: the conversation's text, errors, notices and photos (not tool calls or thinking).
    private var shown: [ChatMessage] {
        store.conversation(agent.id).filter { m in
            ["text", "error", "notice"].contains(m.kind) && !(m.displayContent.isEmpty && m.photos.isEmpty)
        }
    }

    /// Who a shown message is from: the user, this chat's agent, or the other agent that sent it.
    private static func speaker(_ m: ChatMessage) -> String {
        m.kind != "text" ? "-" : m.fromAgentId ?? m.fromAgentName.map { "name:\($0)" } ?? m.role
    }

    var body: some View {
        VStack(spacing: 0) {
            ChatHeader(agent: agent, showsSettings: $showsSettings)
            if let note = nav.chatNote, note.agentId == agent.id {
                ChatNoteBanner(text: note.text) { nav.chatNote = nil }
                .accessibilityIdentifier("chatNote")
                .onDisappear { if nav.chatNote?.agentId == agent.id { nav.chatNote = nil } }  // shown once
                Divider()
            }
            ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if store.historyHasMore[agent.id] == true, !shown.isEmpty {
                        Button("Load earlier messages") { Task { try? await store.loadOlder(agent.id) } }
                            .buttonStyle(.link).foregroundStyle(Theme.accent)
                            .frame(maxWidth: .infinity)
                    }
                    let shown = shown
                    ForEach(Array(shown.enumerated()), id: \.element.id) { i, m in
                        // The sticker shows once per run of replies from the same agent.
                        MessageRow(message: m, agent: agent, showsSticker: i == 0 || Self.speaker(shown[i - 1]) != Self.speaker(m),
                                   reply: store.info?.supports(.reply) == true && m.isQuotable
                                       ? { replyingTo = MessageQuote(of: m, agentName: agent.name) } : nil,
                                   jump: { id in
                                       // One older than the loaded pages can't be scrolled to: say so, as the web does.
                                       if shown.contains(where: { $0.id == id }) {
                                           withAnimation { proxy.scrollTo(id, anchor: .top) }
                                       } else {
                                           nav.chatNote = (agent.id, "That message is no longer in the chat.")
                                       }
                                   })
                            .id(m.id)
                    }
                    ForEach(store.pending(for: agent.id)) { PendingRow(message: $0) }
                    if let a = shownQuickAck { QuickAckRow(text: a) }
                }
                // The web's readable column: 820pt wide at most, centred in a wide window.
                .frame(maxWidth: 820)
                .padding(.horizontal, 24).padding(.vertical, 20)
                .frame(maxWidth: .infinity)
            }
            .defaultScrollAnchor(.bottom)
            .accessibilityIdentifier("messageList")
            }
            .overlay {
                if let loadError, shown.isEmpty {
                    ContentUnavailableView("Couldn't load the chat", systemImage: "exclamationmark.bubble", description: Text(loadError.title))
                }
            }
            if let s = suggested, s.messageId == askingMessage?.id, draft.isEmpty, !s.replies.isEmpty {
                HStack {
                    ForEach(s.replies, id: \.self) { r in
                        Button(r) { draft = r }.buttonStyle(.bordered).controlSize(.small)
                    }
                    Spacer()
                }
                .padding(.horizontal, 12).padding(.top, 8)
                .accessibilityIdentifier("chatSuggestions")
            }
            if let r = route, let target = store.agent(r.agentId) {
                HStack {
                    Label("Send to \(target.name) too?", systemImage: "arrow.turn.up.right").font(.footnote)
                    Spacer()
                    Button("Send") { route = nil; Task { await store.send(r.text, to: r.agentId) } }
                        .buttonStyle(.accent).controlSize(.small)
                        .accessibilityIdentifier("routeSend")
                    Button { route = nil } label: { Image(systemName: "xmark") }
                        .buttonStyle(.bordered).controlSize(.small)
                        .accessibilityLabel("Not now")
                        .accessibilityIdentifier("routeDismiss")
                }
                .padding(.horizontal, 12).padding(.top, 8)
                .accessibilityIdentifier("routeSuggestion")
            }
            Composer(agent: agent, text: $draft, dictated: $draftByVoice, replyingTo: $replyingTo, talk: { talking = true }) { sent in
                route = nil
                quickAck = nil
                guard store.orchestratorId == agent.id else { return }
                let id = agent.id
                let before = store.conversation(id).last?.id ?? 0
                Task {
                    guard let ack = await OnDeviceFeatures.quickAck(for: sent), agent.id == id else { return }
                    VoiceLog.shared.add("ai: quick acknowledgement: \(ack)")
                    quickAck = (id, before, ack, Date())
                }
                guard #available(macOS 26.0, *) else { return }
                Task {
                    guard let target = await OnDeviceFeatures.routeGuess(for: sent, agents: store.agents, orchestratorId: store.orchestratorId),
                          agent.id == id else { return }
                    route = (sent, target.id)
                }
            }
        }
        .sheet(isPresented: $talking) {
            TalkSheet(agentId: agent.id) { id, text in
                nav.openChat(id)
                // Through `nav`, like the voice panel: another agent's chat is a new ChatView, so this one's draft is lost.
                if let text { nav.voiceDraft = (id, text) }
            }
        }
        .onChange(of: nav.voiceDraft?.text, initial: true) {
            guard let d = nav.voiceDraft, d.agentId == agent.id else { return }
            draft = d.text
            draftByVoice = true
            nav.voiceDraft = nil
        }
        .onChange(of: nav.talkIn, initial: true) { _, id in
            guard id == agent.id else { return }
            nav.talkIn = nil
            talking = true
        }
        .task(id: askingMessage?.id) {
            guard let m = askingMessage, suggested?.messageId != m.id else { return }
            let replies = await OnDeviceFeatures.chatReplies(to: m.displayContent)
            guard !Task.isCancelled else { return }
            suggested = (m.id, replies)
        }
        .task(id: agent.id) {
            do { try await store.loadConversation(agent.id); loadError = nil } catch { loadError = APIError.from(error) }
        }
    }
}

private struct MessageRow: View {
    @Environment(LoftStore.self) private var store
    let message: ChatMessage
    /// The chat's agent: the sticker beside its replies.
    let agent: Agent
    /// The first of a run of replies from the same agent: it shows the sticker.
    var showsSticker = true
    /// Reply to this message (nil when it can't be quoted), and jump to the one it quotes.
    var reply: (() -> Void)?
    var jump: (Int64) -> Void = { _ in }

    /// The user's own message. Another agent's handover, delegation or FYI also comes as role user, with who sent
    /// it: that one is incoming, on the left, as on the iPhone and the web.
    private var isUser: Bool { message.role == "user" && message.fromAgentName == nil }

    var body: some View {
        switch message.kind {
        case "error":
            Label(message.displayContent, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(Theme.failureInk)
                .textSelection(.enabled)
        case "notice":
            Text(message.displayContent).font(.footnote).foregroundStyle(.secondary).frame(maxWidth: .infinity)
        default:
            HStack(alignment: .top, spacing: 10) {
                if isUser {
                    Spacer(minLength: 80)
                } else {
                    Group {
                        if !showsSticker {
                            Color.clear
                        } else if let sender {
                            AgentSticker(agent: sender, size: 26)
                        } else {
                            Initials(name: message.fromAgentName ?? "?", size: 26)
                        }
                    }
                    .frame(width: 26, height: 26)
                    .padding(.top, 2)
                }
                VStack(alignment: isUser ? .trailing : .leading, spacing: 4) {
                    if let from = message.fromAgentName {
                        Text(message.isMentionReply ? "\(from) · answering your mention" : "From \(from)")
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundStyle(Theme.accent)
                    }
                    let photos = message.photos.compactMap { store.uploadURL(agentId: message.agentId, file: $0) }
                    if !photos.isEmpty {
                        HStack { ForEach(photos, id: \.self) { Photo(url: $0) } }
                    }
                    if let q = message.replyQuote {
                        QuoteBlock(quote: q) { jump(q.id) }
                    }
                    if !message.displayContent.isEmpty {
                        if isUser {
                            // As on the web: your messages in a faint bubble on the right.
                            Text(MarkdownView.inline(message.displayContent))
                                .font(.system(size: 14))
                                .lineSpacing(3)
                                .textSelection(.enabled)
                                .padding(.horizontal, 14).padding(.vertical, 9)
                                .foregroundStyle(WebTheme.fg)
                                .background(WebTheme.fg.opacity(0.07),
                                            in: UnevenRoundedRectangle(topLeadingRadius: 16, bottomLeadingRadius: 16,
                                                                       bottomTrailingRadius: 6, topTrailingRadius: 16, style: .continuous))
                        } else {
                            // The agent's reply as one card, so each reads as a block.
                            let shape = UnevenRoundedRectangle(topLeadingRadius: 6, bottomLeadingRadius: 16,
                                                               bottomTrailingRadius: 16, topTrailingRadius: 16, style: .continuous)
                            MarkdownView(message.displayContent)
                                .foregroundStyle(WebTheme.fg)
                                .padding(.horizontal, 16).padding(.vertical, 12)
                                .background(WebTheme.surface, in: shape)
                                .overlay(shape.strokeBorder(WebTheme.line))
                        }
                    }
                    Text(message.createdAt, style: .time).font(.system(size: 10.5, design: .monospaced)).foregroundStyle(WebTheme.fg3)
                }
            }
            .contextMenu {
                if let reply {
                    Button { reply() } label: { Label("Reply", systemImage: "arrowshape.turn.up.left") }
                }
                Button { copy() } label: { Label("Copy", systemImage: "doc.on.doc") }
            }
            // Selectable text isn't read by a combined element, so the label says it.
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(spoken)
            .accessibilityAction(named: "Copy") { copy() }
            .accessibilityAction(named: "Reply") { reply?() }
            .accessibilityAction(named: "Go to the quoted message") { if let q = message.replyQuote { jump(q.id) } }
            .accessibilityIdentifier(isUser ? "userMessage" : "agentMessage")
        }
    }

    private func copy() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(message.displayContent, forType: .string)
    }

    /// The agent whose sticker shows: the other agent that sent it, or the chat's own agent.
    private var sender: Agent? {
        if let id = message.fromAgentId { return store.agent(id) }
        if let name = message.fromAgentName { return store.agents.first { $0.name == name } }
        return agent
    }

    private var spoken: String {
        var parts: [String] = []
        if let from = message.fromAgentName { parts.append(message.isMentionReply ? "\(from), answering your mention" : "From \(from)") }
        if !message.photos.isEmpty { parts.append(message.photos.count == 1 ? "Image" : "\(message.photos.count) images") }
        if let q = message.replyQuote { parts.append("Replying to \(q.authorLabel): \(q.excerpt)") }
        parts.append(message.displayContent)
        parts.append(message.createdAt.formatted(date: .omitted, time: .shortened))
        return parts.filter { !$0.isEmpty }.joined(separator: ", ")
    }

}

/// A message on its way, or one that failed with Retry.
/// The on-device model's one-line first reply, styled apart from real replies until one arrives.
private struct QuickAckRow: View {
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(text)
            Label("Quick reply from this Mac · the real one is on its way", systemImage: "sparkles")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .overlay { RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Theme.accent.opacity(0.35), style: StrokeStyle(lineWidth: 1, dash: [4, 3])) }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Quick reply: \(text)")
        .accessibilityIdentifier("quickAck")
    }
}

private struct PendingRow: View {
    @Environment(LoftStore.self) private var store
    let message: PendingMessage

    var body: some View {
        HStack {
            Spacer(minLength: 80)
            VStack(alignment: .trailing, spacing: 4) {
                if let n = message.images?.count, n > 0 {
                    Label(n == 1 ? "Image" : "\(n) images", systemImage: "photo").font(.caption).foregroundStyle(.secondary)
                }
                if !message.text.isEmpty {
                    Text(message.text)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 9)
                        .foregroundStyle(WebTheme.fg.opacity(message.isFailed ? 0.6 : 0.8))
                        .background(WebTheme.fg.opacity(0.05), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                }
                if let failure = message.failure {
                    HStack {
                        Label("Not sent: \(failure)", systemImage: "exclamationmark.circle").foregroundStyle(Theme.failureInk)
                        Button("Retry") { Task { await store.retry(message) } }
                        Button("Discard") { store.discard(message) }
                    }
                    .font(.caption)
                } else {
                    Text("Sending…").font(.caption2).foregroundStyle(.secondary)
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("pendingMessage")
    }
}

private struct Photo: View {
    let url: URL

    var body: some View {
        AsyncImage(url: url) { image in
            image.resizable().scaledToFill()
        } placeholder: {
            Color.secondary.opacity(0.1)
        }
        .frame(width: 180, height: 140)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .accessibilityLabel("Image")
    }
}

/// An image picked or dropped: uploaded to the agent at once, sent with the next message.
private struct Attachment: Identifiable {
    let id = UUID()
    let file: URL
    var uploaded: String?
    var failure: String?
}

/// The message field, with @ to tag an agent and # to tag a spec, issue or pull request, as on the web.
/// Return sends (or picks the first suggestion while one shows); ⌥Return is a new line.
private struct Composer: View {
    @Environment(LoftStore.self) private var store
    let agent: Agent
    @Environment(VoiceSession.self) private var voice
    @Binding var text: String
    /// The message comes from dictation or Talk: its reply is read aloud.
    @Binding var dictated: Bool
    /// The quote above the box, sent with the next message as its `reply_to`.
    @Binding var replyingTo: MessageQuote?
    /// Opens Talk with this agent.
    let talk: () -> Void
    /// Called with the text just sent.
    var sent: (String) -> Void = { _ in }
    @State private var attachments: [Attachment] = []
    @State private var picksFiles = false
    @State private var dictation = SpeechTranscriber()
    @State private var dictationBase = ""
    @State private var holdTimer: Task<Void, Never>?
    @State private var holding = false
    /// The mouse is still down.
    @State private var pressing = false
    /// This press only stopped a dictation: its release isn't a click for Talk.
    @State private var ignoreRelease = false
    @FocusState private var focused: Bool
    @Environment(MacNav.self) private var nav

    private var mentions: [(title: String, agents: [Agent])]? {
        MentionQuery.query(in: text).map { MentionQuery.sections($0, in: store.agents, excluding: agent.name) }
    }

    private var tags: [TagQuery.Row]? {
        TagQuery.query(in: text).map {
            TagQuery.matches(TagQuery.rows(tasks: store.tasks, prs: store.pullRequests, project: agent.project ?? ""), $0)
        }
    }

    private var canSend: Bool {
        store.canWrite && attachments.allSatisfy { $0.uploaded != nil }
            && (!text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let mentions, !mentions.isEmpty {
                Suggestions {
                    ForEach(mentions, id: \.title) { section in
                        Text(section.title).eyebrow()
                        ForEach(section.agents) { a in
                            SuggestionButton(id: "mention-\(a.name)") { text = MentionQuery.insert(a, into: text) } label: {
                                HStack { Initials(name: a.name, lead: a.isLead, size: 20); Text(a.name) }
                            }
                        }
                    }
                }
            } else if let tags, !tags.isEmpty {
                Suggestions {
                    ForEach(tags) { r in
                        SuggestionButton(id: "tag-\(r.tag)") { text = TagQuery.insert(r.tag, into: text) } label: {
                            HStack {
                                Image(systemName: r.kind == .spec ? "doc.text" : r.kind == .pr ? "arrow.triangle.pull" : "checklist")
                                    .foregroundStyle(.secondary)
                                Text(r.tag).monospaced().foregroundStyle(.secondary)
                                Text(r.title).lineLimit(1)
                            }
                        }
                    }
                }
            }
            if let q = replyingTo {
                ReplyBar(quote: q) { replyingTo = nil }
            }
            if !attachments.isEmpty {
                HStack {
                    ForEach(attachments) { a in
                        AttachmentChip(attachment: a) { attachments.removeAll { $0.id == a.id } }
                    }
                }
            }
            HStack(alignment: .bottom, spacing: 8) {
                Button { picksFiles = true } label: { Image(systemName: "paperclip") }
                    .buttonStyle(.borderless)
                    .help("Attach images")
                    .accessibilityLabel("Attach images")
                    .accessibilityIdentifier("attachButton")
                TextField("Message \(agent.name)", text: $text, axis: .vertical)
                    .textFieldStyle(.plain)
                    .lineLimit(1...8)
                    .focused($focused)
                    .onSubmit(submit)
                    // Shift-Return adds a line at the caret instead of sending, as on the web.
                    .onKeyPress(.return, phases: .down) { press in
                        guard press.modifiers.contains(.shift) else { return .ignored }
                        NSApp.sendAction(#selector(NSResponder.insertNewlineIgnoringFieldEditor(_:)), to: nil, from: nil)
                        return .handled
                    }
                    .accessibilityIdentifier("composerField")
                micButton
                Button(action: send) { Image(systemName: "arrow.up.circle.fill").font(.title2).foregroundStyle(canSend ? Theme.accent : WebTheme.fg3) }
                    .buttonStyle(.borderless)
                    .disabled(!canSend)
                    .help("Send (Return) · New line (Shift-Return)")
                    .accessibilityLabel("Send")
                    .accessibilityIdentifier("sendButton")
            }
            // The web's composer: a rounded field on ink-850 with a strong hairline, green when focused.
            .padding(10)
            .background(WebTheme.field, in: RoundedRectangle(cornerRadius: WebTheme.panelRadius, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: WebTheme.panelRadius, style: .continuous)
                .strokeBorder(focused ? Theme.accent.opacity(0.4) : WebTheme.lineStrong))
        }
        .padding(12)
        // Takes the keyboard when the chat opens from code; a sidebar click leaves it there, so ↑/↓ keep moving
        // between sections.
        .onAppear {
            focused = MacSelection.composerTakesKeyboard(
                requested: nav.selection.takeComposerRequest(),
                noTextFieldFocused: !(NSApp.keyWindow?.firstResponder is NSText))
        }
        // The same chat opened again from code: it doesn't appear anew.
        // (A chat for another agent is a new view, which asks in onAppear.)
        .onChange(of: nav.selection.composerRequests) { focused = true }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in
            // Switching away (or a system prompt) can swallow the mouse-up: the mic never stays on behind another app.
            pressing = false
            if dictation.state.isRecording { stopDictation() }
        }
        .onChange(of: dictation.transcript) { _, t in
            guard dictation.state.isRecording, !t.isEmpty else { return }
            text = VoiceText.join([dictationBase, t])
            dictated = true
        }
        .fileImporter(isPresented: $picksFiles, allowedContentTypes: [.image], allowsMultipleSelection: true) { result in
            if case .success(let urls) = result { attach(urls) }
        }
        .dropDestination(for: URL.self) { urls, _ in
            let images = urls.filter { UTType(filenameExtension: $0.pathExtension)?.conforms(to: .image) == true }
            attach(images)
            return !images.isEmpty
        }
    }

    /// Tap for Talk, hold to dictate into the field (let go to stop).
    private var micButton: some View {
        VStack(alignment: .trailing, spacing: 2) {
            Image(systemName: dictation.state.isRecording ? "mic.circle.fill" : "mic")
                .font(dictation.state.isRecording ? .title2 : .body)
                .foregroundStyle(dictation.state.isRecording ? Theme.failure : Color.primary)
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
                .onLongPressGesture(minimumDuration: 60, maximumDistance: 40, perform: {}, onPressingChanged: { pressing in
                    if pressing { pressed() } else { released() }
                })
                .help("Click to talk, hold to dictate")
                .accessibilityLabel("Talk or dictate")
                .accessibilityHint("Click to talk to \(agent.name). Hold to dictate into the message.")
                .accessibilityAddTraits(.isButton)
                .accessibilityAction { talk() }
                .accessibilityIdentifier("micButton")
            if !dictation.state.isRecording, let message = dictation.state.message {
                Text(message).font(.caption2).foregroundStyle(Theme.failureInk).frame(maxWidth: 220, alignment: .trailing)
                    .accessibilityIdentifier("dictationMessage")
                if case .denied = dictation.state {
                    Button("Open System Settings") { NSWorkspace.shared.open(TalkPermission.settingsURL(for: message)) }
                        .font(.caption2).buttonStyle(.link).foregroundStyle(Theme.accent)
                        .accessibilityIdentifier("dictationOpenSettings")
                }
            }
        }
    }

    private func pressed() {
        if dictation.state.isRecording {
            // A click while dictating stops it, so a lost mouse-up (a permission prompt) can't leave it on.
            stopDictation()
            ignoreRelease = true
            return
        }
        pressing = true
        holdTimer?.cancel()
        holdTimer = Task {
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled else { return }
            holding = true
            dictationBase = text
            await dictation.start()
            // Let go while the permission prompt or the microphone was starting.
            if !pressing { stopDictation() }
        }
    }

    /// Let go: after the hold time that ends the dictation, before it the press was a click, which opens Talk.
    private func released() {
        pressing = false
        if ignoreRelease { ignoreRelease = false; return }
        holdTimer?.cancel()
        holdTimer = nil
        if holding {
            stopDictation()
        } else {
            talk()
        }
    }

    private func stopDictation() {
        holding = false
        dictation.stop()
    }

    /// Return: picks the first suggestion while the @ or # list shows, otherwise sends.
    private func submit() {
        if let a = mentions?.first?.agents.first {
            text = MentionQuery.insert(a, into: text)
        } else if let r = tags?.first {
            text = TagQuery.insert(r.tag, into: text)
        } else {
            send()
        }
        focused = true
    }

    private func send() {
        guard canSend else { return }
        let message = text, images = attachments.compactMap(\.uploaded)
        let meta = MessageQuote.meta(nil, replyingTo: replyingTo)
        text = ""
        attachments = []
        replyingTo = nil
        sent(message)
        if dictated {
            dictated = false
            Task { await store.sendByVoice(message, to: agent.id, meta: meta, images: images, reads: voice.readReplies) { answer, id in
                _ = await voice.speaker.readReply(answer, messageId: id, isActive: { true })
            } }
        } else {
            Task { await store.send(message, to: agent.id, meta: meta, images: images) }
        }
    }

    private func attach(_ urls: [URL]) {
        for url in urls {
            let a = Attachment(file: url)
            attachments.append(a)
            Task {
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                do {
                    let name = try await store.upload(url, to: agent.id) { _ in }
                    if let i = attachments.firstIndex(where: { $0.id == a.id }) { attachments[i].uploaded = name }
                } catch {
                    if let i = attachments.firstIndex(where: { $0.id == a.id }) { attachments[i].failure = APIError.from(error).title }
                }
            }
        }
    }
}

private struct Suggestions<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 4) { content }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
        }
        .frame(maxHeight: 220)
        .fixedSize(horizontal: false, vertical: true)
        .background(WebTheme.field, in: RoundedRectangle(cornerRadius: WebTheme.rowRadius, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: WebTheme.rowRadius, style: .continuous).strokeBorder(WebTheme.lineStrong))
        .accessibilityIdentifier("suggestions")
    }
}

private struct SuggestionButton<Label: View>: View {
    let id: String
    let action: () -> Void
    @ViewBuilder let label: Label

    var body: some View {
        Button(action: action) { label.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle()) }
            .buttonStyle(.plain)
            .padding(.vertical, 2)
            .accessibilityIdentifier(id)
    }
}

private struct AttachmentChip: View {
    let attachment: Attachment
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            if let failure = attachment.failure {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Theme.failure).help(failure)
            } else if attachment.uploaded == nil {
                ProgressView().controlSize(.small)
            } else {
                Image(systemName: "photo")
            }
            Text(attachment.file.lastPathComponent).lineLimit(1).frame(maxWidth: 140)
            Button(action: remove) { Image(systemName: "xmark.circle.fill") }
                .buttonStyle(.borderless)
                .accessibilityLabel("Remove \(attachment.file.lastPathComponent)")
        }
        .font(.system(size: 11, design: .monospaced))
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .foregroundStyle(WebTheme.fg2)
        .overlay(Capsule().strokeBorder(WebTheme.line))
    }
}

/// The quote above the message box while replying, with ✕ to remove it. Plain text, never markdown.
private struct ReplyBar: View {
    let quote: MessageQuote
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "arrowshape.turn.up.left").foregroundStyle(WebTheme.fg3)
            VStack(alignment: .leading, spacing: 1) {
                Text("Replying to \(quote.authorLabel)").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.accent)
                Text(verbatim: quote.excerpt).font(.system(size: 12)).foregroundStyle(WebTheme.fg2).lineLimit(1)
            }
            Spacer(minLength: 0)
            Button(action: remove) { Image(systemName: "xmark.circle.fill").font(.system(size: 14)).contentShape(Rectangle()) }
                .buttonStyle(.plain)
                .foregroundStyle(WebTheme.fg3)
                .help("Remove the quote")
                .accessibilityLabel("Remove the quote")
                .accessibilityIdentifier("replyQuoteRemove")
        }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(WebTheme.line))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Replying to \(quote.authorLabel): \(quote.excerpt)")
        .accessibilityIdentifier("replyQuote")
    }
}

/// The message a reply quotes, above its text: who wrote it and a line or two, as plain text. A click jumps to it.
private struct QuoteBlock: View {
    let quote: MessageQuote
    let jump: () -> Void

    var body: some View {
        Button(action: jump) {
            HStack(spacing: 8) {
                RoundedRectangle(cornerRadius: 2).fill(Theme.accent).frame(width: 3)
                VStack(alignment: .leading, spacing: 1) {
                    Text(quote.authorLabel).font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.accent)
                    Text(verbatim: quote.excerpt).font(.system(size: 12)).foregroundStyle(WebTheme.fg2)
                        .lineLimit(2).multilineTextAlignment(.leading)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(WebTheme.fg.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Go to the quoted message")
        .accessibilityIdentifier("messageQuote")
    }
}

/// The line at the top of a chat that was opened here because Herdr couldn't (`MacNav.chatNote`).
struct ChatNoteBanner: View {
    let text: String
    let dismiss: () -> Void

    var body: some View {
        HStack {
            Text(text).font(.callout).foregroundStyle(Theme.attentionInk)
            Spacer()
            Button(action: dismiss) { Image(systemName: "xmark") }
                .buttonStyle(.borderless).accessibilityLabel("Dismiss")
        }
        .padding(.horizontal, 24).padding(.vertical, 8)
    }
}

#if DEBUG
/// `-render-chat-note <file.png>`: the banner as a fallback from Herdr shows it, drawn to a PNG, then quits. For screenshots.
@MainActor
func renderChatNote(to file: String) {
    let view = ChatNoteBanner(text: "Opened here: Herdr isn't running.") {}
        .frame(width: 720).background(Color(white: 0.07)).environment(\.colorScheme, .dark)
    let renderer = ImageRenderer(content: view)
    renderer.scale = 2
    if let tiff = renderer.nsImage?.tiffRepresentation, let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) {
        try? png.write(to: URL(fileURLWithPath: file))
    }
    NSApp.terminate(nil)
}

/// `-render-reply <folder>`: a reply as the chat shows it (the quoted message, the reply with its quote) and the quote
/// above the message box, drawn to dark.png and light.png, then quits. For screenshots (spec #572).
@MainActor
func renderReply(to folder: String) async {
    // The fixture's Lead chat: its question, a reply to it (stored as the server does), and its next question.
    let api = FixtureAPI()
    guard let agent = try? await api.agents().first(where: { $0.id == "orch" }),
          let history = try? await api.messages(agentId: "orch", limit: 50, before: nil),
          let asked = history.first(where: { $0.id == 2 }),
          let reply = try? await api.sendMessage(agentId: "orch", text: "Wait for QA before the iOS part.", clientId: "r",
                                                 meta: ["reply_to": .number(2)]).message,
          let next = history.last(where: { $0.role == "assistant" && $0.kind == "text" && $0.id != 2 })
    else { return NSApp.terminate(nil) }
    let store = LoftStore(api: api)
    for (name, scheme) in [("dark", ColorScheme.dark), ("light", .light)] {
        NSAppearance.current = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        let view = VStack(alignment: .leading, spacing: 12) {
            MessageRow(message: asked, agent: agent)
            MessageRow(message: reply, agent: agent)
            MessageRow(message: next, agent: agent)
            ReplyBar(quote: MessageQuote(of: next, agentName: agent.name)) {}
        }
        .padding(24).frame(width: 720)
        .background(WebTheme.canvas).environment(\.colorScheme, scheme).environment(store)
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        if let tiff = renderer.nsImage?.tiffRepresentation, let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) {
            try? png.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
        }
    }
    NSApp.terminate(nil)
}
#endif
