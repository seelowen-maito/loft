import AppKit
import LoftKit
import SwiftUI
import UniformTypeIdentifiers

/// An agent's chat: its messages as they arrive, and the composer.
struct ChatView: View {
    @Environment(LoftStore.self) private var store
    @Environment(MacNav.self) private var nav
    let agent: Agent
    @State private var loadError: APIError?
    @State private var draft = ""
    @State private var talking = false
    /// The text came from Talk, so the reply is read aloud once it's sent.
    @State private var draftByVoice = false
    /// The on-device model's replies to the agent's latest message, when it ends on a question.
    @State private var suggested: (messageId: Int64, replies: [String])?
    /// "Send to …?": the model thinks the message just sent to the Lead was meant for another agent.
    @State private var route: (text: String, agentId: String)?

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

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    if store.historyHasMore[agent.id] == true, !shown.isEmpty {
                        Button("Load earlier messages") { Task { try? await store.loadOlder(agent.id) } }
                            .buttonStyle(.link).foregroundStyle(Theme.accent)
                            .frame(maxWidth: .infinity)
                    }
                    ForEach(shown) { MessageRow(message: $0) }
                    ForEach(store.pending(for: agent.id)) { PendingRow(message: $0) }
                }
                .padding(16)
            }
            .defaultScrollAnchor(.bottom)
            .accessibilityIdentifier("messageList")
            .overlay {
                if let loadError, shown.isEmpty {
                    ContentUnavailableView("Couldn't load the chat", systemImage: "exclamationmark.bubble", description: Text(loadError.title))
                }
            }
            Divider()
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
            Composer(agent: agent, text: $draft, dictated: $draftByVoice, talk: { talking = true }) { sent in
                route = nil
                guard store.orchestratorId == agent.id, #available(macOS 26.0, *) else { return }
                let id = agent.id
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
                if let text {
                    draft = text
                    draftByVoice = true
                }
            }
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
            HStack(alignment: .top) {
                if isUser { Spacer(minLength: 80) }
                VStack(alignment: isUser ? .trailing : .leading, spacing: 4) {
                    if let from = message.fromAgentName {
                        Text(message.isMentionReply ? "\(from) · answering your mention" : "From \(from)")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.tint)
                    }
                    let photos = message.photos.compactMap { store.uploadURL(agentId: message.agentId, file: $0) }
                    if !photos.isEmpty {
                        HStack { ForEach(photos, id: \.self) { Photo(url: $0) } }
                    }
                    if !message.displayContent.isEmpty {
                        Text(Self.markdown(message.displayContent))
                            .textSelection(.enabled)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .foregroundStyle(isUser ? Theme.onAccent : Color.primary)
                            .background(isUser ? Theme.accent : Color.secondary.opacity(0.12),
                                        in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                    }
                    Text(message.createdAt, style: .time).font(.caption2).foregroundStyle(.secondary)
                }
                if !isUser { Spacer(minLength: 80) }
            }
            // Selectable text isn't read by a combined element, so the label says it.
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(spoken)
            .accessibilityIdentifier(isUser ? "userMessage" : "agentMessage")
        }
    }

    private var spoken: String {
        var parts: [String] = []
        if let from = message.fromAgentName { parts.append(message.isMentionReply ? "\(from), answering your mention" : "From \(from)") }
        if !message.photos.isEmpty { parts.append(message.photos.count == 1 ? "Image" : "\(message.photos.count) images") }
        parts.append(message.displayContent)
        parts.append(message.createdAt.formatted(date: .omitted, time: .shortened))
        return parts.filter { !$0.isEmpty }.joined(separator: ", ")
    }

    static func markdown(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
    }
}

/// A message on its way, or one that failed with Retry.
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
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .foregroundStyle(Theme.onAccent)
                        .background(Theme.accent.opacity(message.isFailed ? 0.4 : 0.7), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
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
                        Text(section.title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
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
                    .accessibilityIdentifier("composerField")
                micButton
                Button(action: send) { Image(systemName: "arrow.up.circle.fill").font(.title2) }
                    .buttonStyle(.borderless)
                    .disabled(!canSend)
                    .help("Send (Return)")
                    .accessibilityLabel("Send")
                    .accessibilityIdentifier("sendButton")
            }
        }
        .padding(12)
        .onAppear { focused = true }
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
        text = ""
        attachments = []
        sent(message)
        if dictated {
            dictated = false
            Task { await store.sendByVoice(message, to: agent.id, images: images, reads: voice.readReplies) { answer, id in
                _ = await voice.speaker.readReply(answer, messageId: id, isActive: { true })
            } }
        } else {
            Task { await store.send(message, to: agent.id, images: images) }
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
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10))
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
        .font(.caption)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(.background.secondary, in: Capsule())
    }
}
