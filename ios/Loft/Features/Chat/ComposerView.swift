import SwiftUI

/// Multiline composer with a send button and a microphone that transcribes into the text field.
/// Dictation never sends because you went quiet. The mic stays on through pauses and keeps adding to
/// the same message; "send" sends, "review" keeps the text to edit, "cancel" throws it away. After
/// a few seconds of quiet it asks "Would you like to submit this?" (aloud and on screen) and keeps
/// listening: yes cleans the text up on the Mac and sends it with a short Undo, no keeps it, more
/// words carry on the message (VoiceSend).
struct ComposerView: View {
    @Environment(LoftStore.self) private var store
    @Environment(Speaker.self) private var speaker
    @Binding var text: String
    let recipient: String
    /// Who an auto-send goes to.
    let agentId: String
    let isOrchestrator: Bool
    /// Set by the parent to start or cancel dictation (the grill's hands-free loop); reset to nil once handled.
    var control: Binding<ComposerControl?> = .constant(nil)
    /// Each new piece of dictated text, without what was typed before dictation started.
    var onDictation: ((String) -> Void)?
    /// While true (e.g. "Skip this question?" is up), the app doesn't ask to submit.
    var holdAutoSend: () -> Bool = { false }
    var dictateOnAppear = false
    /// Long-pressing the mic starts a full voice conversation instead of dictation.
    var startVoice: (() -> Void)?
    /// The other way round (an agent's chat, #340): tapping the mic opens Talk, holding it dictates into the field.
    var tapOpensTalk = false
    /// A dictation was just sent by voice ("send", or yes to the question), so the parent can read the reply and listen again.
    var onVoiceSent: (() -> Void)?
    /// "Stop" said on its own ends dictation without sending. Nil: "stop" is plain text (the grill reads it itself).
    var onVoiceStop: (() -> Void)?
    /// Shows "Attach an issue" next to @ (the chat keeps the attached task).
    var attachTask: (() -> Void)?
    /// Shows the paperclip: photos and a video picked or taken are uploaded to this agent and kept here until sent
    /// (the chat reads them when `send` is called).
    var videoAgentId: String?
    var attachments: Binding<[AttachmentDraft]> = .constant([])
    /// The text, and whether it went out as a voice turn (dictated, or sent by voice) rather than typed.
    let send: (String, Bool) -> Void

    private enum Phase: Equatable {
        case idle
        /// Mic on, adding to the message.
        case listening
        /// Saying "Would you like to submit this?" (mic off meanwhile, so it doesn't hear itself).
        case asking
        /// Mic on, waiting for yes / no / more words.
        case awaitingAnswer
        /// Tidying the text up on the Mac before sending.
        case cleaning
    }

    @State private var speech = SpeechTranscriber()
    /// The text that was in the field when this recognition run started; the transcript is appended after it.
    @State private var prefix = ""
    @FocusState private var focused: Bool
    /// Survives the composer being recreated, so returning to the chat doesn't start dictating again.
    @State private var didAutoDictate = false
    /// Set by the long press so the tap that ends it doesn't also toggle dictation.
    @State private var longPressed = false
    @State private var phase: Phase = .idle
    @State private var prompt = VoiceSend.Prompt(startedAt: Date())
    @State private var lastHeard = Date()
    @State private var micStarted = Date()
    /// The transcript's words last time, so a reformatting (punctuation, capitals) isn't taken for new speech.
    @State private var lastWords = ""
    /// The transcript already checked for a command or an answer.
    @State private var evaluated = ""
    /// The transcript at the latest gap: words after it are one utterance ("…[pause] send").
    @State private var pauseBase = ""
    @State private var silenceWatch: Task<Void, Never>?
    /// Whether the composer is on screen: an auto-send that ends after you left sends only the message.
    @State private var shown = Shown()
    /// The text in the field is going out because of a voice yes / "send".
    @State private var voiceSending = false
    /// The text in the field was (at least partly) dictated: sending it, even with a tap, is a voice turn, so
    /// the reply is read aloud. Cleared when the field is emptied.
    @State private var dictated = false
    /// What was said before "Make it a brief", and the brief as it went in: "Use what I said" puts it back.
    @State private var briefOf: (original: String, brief: String)?
    @State private var briefing = false
    /// Why there's no brief ("Already short", "Couldn't make a brief, kept your words"), until the text changes.
    @State private var briefNote: String?
    @AppStorage(Prefs.voiceBrief) private var offerBrief = true

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }
    /// A photo or video still reading or uploading, or one that failed, holds the message back until it's up or
    /// removed; so do photos over the message limit together.
    private var attachmentsHold: Bool { attachments.wrappedValue.contains { $0.uploaded == nil } || photosRefusal != nil }
    private var photosRefusal: String? {
        let a = attachments.wrappedValue
        return PhotoAttachment.refusal(bytes: a.filter { !$0.isVideo }.map(\.bytes), video: a.contains(where: \.isVideo))
    }
    private var canSend: Bool { (!trimmed.isEmpty || !attachments.wrappedValue.isEmpty) && !attachmentsHold && store.canWrite }
    /// "Sending to … · Undo" after yes, with the cleaned text: held by the store, so it still goes if you leave.
    private var pending: LoftStore.VoiceAutoSend? { store.voiceAutoSend.flatMap { $0.agentId == agentId ? $0 : nil } }
    private var micOn: Bool { phase == .listening || phase == .awaitingAnswer || phase == .asking }

    var body: some View {
        VStack(spacing: 6) {
            if let p = pending {
                AutoSendBar(recipient: recipient, deadline: p.deadline, undo: undoSend, sendNow: { store.sendVoiceNow(p.id) })
            } else if phase == .cleaning {
                Label("Tidying up what you said…", systemImage: "wand.and.stars")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("voiceCleaning")
            } else if phase == .asking || phase == .awaitingAnswer {
                SubmitPromptBar(question: VoiceSend.question(for: trimmed), yes: { answered(.yes, by: "tap") },
                                no: { answered(.no, by: "tap") })
            }
            if pending == nil, phase == .idle { briefBar }
            if let message = speech.state.message {
                Label(message, systemImage: speech.state.isRecording ? "waveform" : "mic.slash")
                    .font(.caption)
                    .foregroundStyle(speech.state.isRecording ? Theme.failure : .secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("speechStatus")
            }
            if !store.canWrite {
                Text("Offline. Messages can be sent once you reconnect.")
                    .font(.caption)
                    .foregroundStyle(Theme.attention)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            let photos = attachments.wrappedValue.filter { !$0.isVideo }
            if !photos.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) { ForEach(photos) { p in PhotoChip(draft: p) { remove(p) } } }
                }
                .accessibilityIdentifier("draftPhotos")
                if let why = photosRefusal {
                    Text(why).font(.caption).foregroundStyle(Theme.failure).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            if let v = attachments.wrappedValue.first(where: \.isVideo) {
                VideoChip(draft: v) { remove(v) }
            }
            if let q = MentionQuery.query(in: text) {
                MentionPicker(sections: MentionQuery.sections(q, in: store.agents, excluding: recipient)) { a in
                    text = MentionQuery.insert(a, into: text)
                }
            }
            if let q = TagQuery.query(in: text) {
                let project = store.agents.first { $0.name == recipient }?.project ?? ""
                TagPicker(rows: TagQuery.matches(TagQuery.rows(tasks: store.tasks, prs: store.pullRequests, project: project), q)) { r in
                    text = TagQuery.insert(r.tag, into: text)
                }
            }
            HStack(alignment: .bottom, spacing: 8) {
                Button { text = MentionQuery.start(text) } label: {
                    Image(systemName: "at").font(.system(size: 18, weight: .semibold)).frame(width: 32, height: 44)
                }
                .foregroundStyle(.secondary)
                .accessibilityLabel("Mention an agent")
                .accessibilityHint("Shows the agents to pick from")
                .accessibilityIdentifier("mentionButton")
                if let attachTask {
                    Button(action: attachTask) {
                        Image(systemName: "checklist").font(.system(size: 17, weight: .semibold)).frame(width: 30, height: 44)
                    }
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Attach an issue")
                    .accessibilityHint("The agent gets the issue with your message")
                    .accessibilityIdentifier("attachTaskButton")
                }
                if let agentId = videoAgentId {
                    let upload = { (file: URL, progress: @escaping @Sendable (Double) -> Void) in
                        try await store.upload(file, to: agentId, progress: progress)
                    }
                    AttachMenu(photosLeft: max(0, PhotoAttachment.maxPerMessage - attachments.wrappedValue.count)) { loads in
                        for load in loads.prefix(PhotoAttachment.maxPerMessage - attachments.wrappedValue.count) {
                            let draft = AttachmentDraft(video: false)
                            draft.start(load: load, upload: upload)
                            attachments.wrappedValue.append(draft)
                        }
                    } pickedVideo: { load in
                        // At most one video per message: a new one takes the old one's place.
                        attachments.wrappedValue.first(where: \.isVideo).map(remove)
                        let draft = AttachmentDraft(video: true)
                        draft.start(load: load, upload: upload)
                        attachments.wrappedValue.append(draft)
                    }
                }
                TextField(isOrchestrator ? "Ask \(recipient) to plan or delegate…" : "Message \(recipient)…", text: $text, axis: .vertical)
                    .lineLimit(1...6)
                    .focused($focused)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
                    .background(Theme.card, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                    .accessibilityIdentifier("composerField")
                Button {
                    if longPressed { longPressed = false } else if talks, !micOn { openTalk() } else { toggleMic() }
                } label: {
                    if talks {
                        // Home's Talk button, at the composer's size.
                        Image(systemName: micOn ? "stop.fill" : "mic.fill")
                            .font(.body.weight(.semibold))
                            .foregroundStyle(Theme.onAccent)
                            .frame(width: 32, height: 32)
                            .background(micOn ? Theme.failure : Theme.accent, in: Circle())
                    } else {
                        Image(systemName: speech.state.isRecording ? "stop.circle.fill" : "mic.circle.fill")
                            .font(.system(size: 32))
                            .symbolRenderingMode(.hierarchical)
                            .foregroundStyle(speech.state.isRecording ? Theme.failure : .secondary)
                    }
                }
                .simultaneousGesture(LongPressGesture(minimumDuration: 0.5).onEnded { _ in
                    guard startVoice != nil, !speech.state.isRecording, !micOn else { return }
                    longPressed = true
                    if talks { toggleMic() } else { openTalk() }
                })
                .sensoryFeedback(.impact(weight: .medium), trigger: longPressed) { _, new in new }
                .accessibilityLabel(micOn ? "Stop dictation" : talks ? "Talk" : "Dictate")
                .accessibilityHint(micOn || startVoice == nil ? ""
                                   : talks ? "Opens a voice conversation. Touch and hold to dictate into the message instead."
                                   : "Touch and hold for a voice conversation")
                .accessibilityActions {
                    if !micOn, startVoice != nil {
                        if talks {
                            Button("Dictate into the message") { toggleMic() }
                        } else {
                            Button("Voice conversation") { openTalk() }
                        }
                    }
                }
                .accessibilityIdentifier("micButton")
                Button(action: sendNow) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 32))
                        .foregroundStyle(canSend ? Theme.accent : .secondary.opacity(0.5))
                }
                .disabled(!canSend)
                .accessibilityLabel("Send")
                .accessibilityIdentifier("sendButton")
                .sensoryFeedback(.impact(weight: .light), trigger: store.pending.count) { old, new in new > old }
            }
            Text("@ agents and files · # specs, issues and PRs")
                .font(.caption2).foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("composerHint")
        }
        .padding(.horizontal, Theme.gutter)
        .padding(.top, 8)
        .padding(.bottom, 8)
        .background(.bar)
        .onChange(of: speech.transcript) { _, t in
            // The transcript only changes while dictating (late results are dropped), so edits made after stopping stay.
            guard !t.isEmpty, phase == .listening || phase == .awaitingAnswer else { return }
            let words = VoiceText.normalize(t)
            // The recognizer often rewrites what it already has (punctuation, a final pass): not new speech.
            guard words != lastWords else { return }
            lastWords = words
            lastHeard = Date()
            prompt.heardWords(at: lastHeard)
            if phase == .awaitingAnswer {
                // A few words could be the answer; more is the message going on.
                if words.split(separator: " ").count > 3 { resumeMessage(t, why: "more words after the question") }
                return
            }
            text = prefix.isEmpty ? t : prefix + " " + t
            dictated = true
            onDictation?(t)
        }
        .onChange(of: speech.state.isRecording) { _, recording in
            // Tapping stop, an interruption or the recognizer giving up: keep the words, send nothing.
            guard !recording, phase == .listening || phase == .awaitingAnswer else { return }
            VoiceLog.shared.add("mic stopped (tap, interruption or recognizer error): kept, nothing sent")
            endDictation()
        }
        .onChange(of: text) { old, t in
            if t.isEmpty { dictated = false; briefOf = nil }
            if briefNote != nil, !briefing, old != t { briefNote = nil }
        }
        .onChange(of: focused) { _, f in
            // Starting to edit stops listening and any pending send.
            guard f, phase != .idle || pending != nil else { return }
            VoiceLog.shared.add("field tapped: mic off, kept for editing")
            cancelAutoSend()
            endDictation()
        }
        .onChange(of: control.wrappedValue) { _, c in
            guard let c else { return }
            control.wrappedValue = nil
            switch c {
            case .listen:
                if phase == .idle, pending == nil { startDictation() }
            case .cancelDictation:
                // A spoken grill command: drop its words from the field.
                VoiceLog.shared.add("spoken grill command: dictation dropped")
                cancelAutoSend()
                endDictation()
                text = prefix
            case .stopListening:
                // The interview ended (or the chat asked): mic off, nothing sent, the words stay.
                guard phase != .idle || pending != nil else { return }
                VoiceLog.shared.add("asked to stop listening: mic off, nothing sent")
                cancelAutoSend()
                endDictation()
            }
        }
        .onAppear {
            shown.on = true
            // Back during the Undo window: the bar with the time left, and the message in the field.
            if let p = pending {
                text = p.text
                voiceSending = true
                scheduleSend(p.text, in: max(0, p.deadline.timeIntervalSinceNow))
            }
        }
        .onDisappear {
            // A pending auto-send still goes: the bar said it would.
            shown.on = false
            endDictation()
        }
        .task {
            #if DEBUG
            // `-fixture-dictation "<text>"`: as if that was just dictated (UI tests, screenshots). A command in it
            // acts; otherwise it's as if you then went quiet, and the question shows (without the mic or speech).
            if let heard = FixtureAPI.launchText("fixture-dictation"), !didAutoDictate {
                // Only counts once applied: if the composer goes away within the second (a re-layout), the next appearance runs it.
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                didAutoDictate = true
                prefix = ""
                let r = VoiceSend.read(heard)
                text = r.text
                dictated = true
                if let c = r.command { act(on: c, text: r.text) } else { phase = .awaitingAnswer }
                return
            }
            #endif
            guard dictateOnAppear, !didAutoDictate else { return }
            didAutoDictate = true
            if phase == .idle { startDictation() }
        }
    }

    // MARK: - Brief

    /// Under a long dictation: "Make it a brief" (Claude on the Mac); after one, "Use what I said". Nothing here sends.
    @ViewBuilder private var briefBar: some View {
        if briefing {
            Label("Writing a brief on the Mac…", systemImage: "text.badge.checkmark")
                .font(.footnote).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("briefWorking")
        } else if briefOf != nil {
            HStack {
                Label("Brief of what you said", systemImage: "text.badge.checkmark").font(.footnote).foregroundStyle(.secondary)
                Spacer()
                Button("Use what I said", action: useOriginal).font(.footnote).frame(minHeight: 44)
                    .accessibilityIdentifier("briefUndo")
            }
        } else if let note = briefNote {
            Label(note, systemImage: "info.circle").font(.footnote).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("briefNote")
        } else if offerBrief, dictated, store.supportsBrief, store.canWrite,
                  trimmed.split(separator: " ").count >= LoftStore.briefMinWords {
            HStack {
                Button(action: makeBrief) { Label("Make it a brief", systemImage: "text.badge.checkmark") }
                    .font(.footnote).frame(minHeight: 44)
                    .accessibilityHint("Claude on your Mac rewrites it shorter. Your words stay one tap away; nothing is sent")
                    .accessibilityIdentifier("briefMake")
                Spacer()
            }
        }
    }

    private func makeBrief() {
        let original = text
        briefing = true
        VoiceLog.shared.add("brief asked for \(VoiceLog.clip(original))")
        Task {
            let r = await store.brief(original)
            briefing = false
            // Changed while it was written (typed, dictated, sent): leave it alone.
            guard text == original else { return VoiceLog.shared.add("brief dropped, the text changed meanwhile") }
            VoiceLog.shared.add("brief: \(r.log)")
            guard let b = r.brief else { briefNote = r.note; return }
            briefOf = (original, b)
            text = b
        }
    }

    /// Back to the words as said, plus anything added after the brief.
    private func useOriginal() {
        guard let b = briefOf else { return }
        let added = text.hasPrefix(b.brief) ? String(text.dropFirst(b.brief.count)) : ""
        text = b.original + added
        briefOf = nil
        VoiceLog.shared.add("brief undone, back to what you said")
    }

    // MARK: - Dictation

    private var talks: Bool { tapOpensTalk && startVoice != nil }

    private func openTalk() {
        focused = false
        startVoice?()
    }

    private func toggleMic() {
        if micOn {
            VoiceLog.shared.add("mic tapped off: kept, nothing sent")
            endDictation()
        } else {
            cancelAutoSend()
            startDictation()
        }
    }

    private func startDictation() {
        prefix = trimmed
        focused = false
        prompt = VoiceSend.Prompt(startedAt: Date())
        micStarted = Date()
        startRecognition(.listening)
        silenceWatch?.cancel()
        silenceWatch = Task { await watch() }
    }

    /// A recognition run on top of what's in the field.
    private func startRecognition(_ next: Phase) {
        lastWords = ""
        evaluated = ""
        pauseBase = ""
        lastHeard = Date()
        phase = next
        Task { await speech.start() }
    }

    /// Mic off; whatever was said stays in the field.
    private func endDictation() {
        silenceWatch?.cancel()
        silenceWatch = nil
        phase = .idle
        speech.stop()
        if speaker.isSpeaking && !speaker.interruptible { speaker.stop() }
    }

    /// Every quarter second while dictating: act on a settled command or answer, ask after a quiet spell,
    /// and turn the mic off when nothing comes.
    private func watch() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(250))
            let now = Date()
            let t = speech.transcript
            let quiet = now.timeIntervalSince(lastHeard)
            switch phase {
            case .listening:
                guard speech.state.isRecording else { continue }
                if t != evaluated, !t.isEmpty, quiet >= VoiceSend.settle {
                    evaluated = t
                    let r = VoiceSend.command(in: t, sincePause: pauseBase)
                    if let c = r.command {
                        act(on: c, text: [prefix, r.text].filter { !$0.isEmpty }.joined(separator: " "))
                        continue
                    }
                    if r.saidToFinish {
                        // "That's it", "pronto": done talking, so ask now.
                        text = [prefix, r.text].filter { !$0.isEmpty }.joined(separator: " ")
                        VoiceLog.shared.add("heard an end phrase: asking now")
                        await ask()
                        continue
                    }
                    if VoiceSend.endsOpen(t) {
                        prompt.leftOpen()
                        VoiceLog.shared.add("stopped mid-sentence: asking after \(Int(VoiceSend.askAfterOpenEnding)) s")
                    }
                }
                if quiet >= VoiceSend.utteranceGap { pauseBase = t }
                if trimmed.isEmpty, now.timeIntervalSince(micStarted) >= VoiceSend.noSpeechTimeout {
                    VoiceLog.shared.add("nothing heard for \(Int(VoiceSend.noSpeechTimeout)) s: mic off")
                    endDictation()
                    return
                }
                if prompt.shouldAsk(hasText: !trimmed.isEmpty, now: now) {
                    if holdAutoSend() { continue }
                    VoiceLog.shared.add("quiet \(String(format: "%g", prompt.wait)) s after \(VoiceLog.clip(trimmed)): asking")
                    await ask()
                }
            case .awaitingAnswer:
                if t.isEmpty || t == evaluated {
                    if now.timeIntervalSince(lastHeard) >= VoiceSend.noSpeechTimeout {
                        VoiceLog.shared.add("no answer for \(Int(VoiceSend.noSpeechTimeout)) s: mic off, kept, nothing sent")
                        endDictation()
                        return
                    }
                    continue
                }
                guard quiet >= VoiceSend.settle else { continue }
                evaluated = t
                let answer = await OnDeviceFeatures.submitAnswer(t, question: VoiceSend.question(for: trimmed))
                // More was said meanwhile, or it was answered another way: the next round decides.
                guard phase == .awaitingAnswer, speech.transcript == t else { continue }
                if let a = answer {
                    answered(a, by: "voice “\(t)”")
                } else if let c = VoiceSend.read(t).command, c != .send, c != .review {
                    act(on: c, text: trimmed)
                } else {
                    resumeMessage(t, why: "not an answer (“\(VoiceLog.clip(t))”)")
                }
            case .idle, .asking, .cleaning:
                return
            }
        }
    }

    /// "Would you like to submit this?" aloud and on screen, then listen for the answer. The mic is off
    /// while it speaks so the question isn't heard as part of the message.
    private func ask() async {
        prompt.didAsk()
        phase = .asking
        prefix = trimmed
        speech.stop()
        let q = VoiceSend.question(for: trimmed)
        if UIAccessibility.isVoiceOverRunning {
            UIAccessibility.post(notification: .announcement, argument: q)
            try? await Task.sleep(for: .seconds(2))
        } else {
            await speaker.speak(q)
        }
        guard phase == .asking else { return }
        VoiceLog.shared.add("asked; listening for yes / no / more")
        startRecognition(.awaitingAnswer)
    }

    /// The question went unanswered and the words go on: back to the message, which grows.
    private func resumeMessage(_ t: String, why: String) {
        VoiceLog.shared.add("question dismissed: \(why)")
        phase = .listening
        evaluated = ""
        pauseBase = ""
        text = prefix.isEmpty ? t : prefix + " " + t
        dictated = true
        onDictation?(t)
    }

    private func answered(_ a: VoiceSend.Answer, by how: String) {
        VoiceLog.shared.add("answer \(a) by \(how)")
        switch a {
        case .yes: submit(text: trimmed, withUndo: true)
        case .no:
            VoiceLog.shared.add("kept for review: mic off")
            endDictation()
        case .discard: act(on: .discard, text: trimmed)
        }
    }

    /// A spoken command, settled.
    private func act(on c: VoiceSend.Command, text message: String) {
        switch c {
        case .send:
            VoiceLog.shared.add("heard “send”: sending")
            submit(text: message, withUndo: false)
        case .review:
            VoiceLog.shared.add("heard “review”: mic off, kept for editing")
            text = message
            endDictation()
        case .stop where onVoiceStop != nil:
            VoiceLog.shared.add("heard “stop”: mic off, nothing sent")
            text = prefix
            endDictation()
            onVoiceStop?()
        case .stop:
            break
        case .discard:
            VoiceLog.shared.add("heard “cancel”: dictation thrown away, mic off")
            text = ""
            endDictation()
        case .startOver:
            VoiceLog.shared.add("heard “start over”: cleared, still listening")
            text = ""
            prefix = ""
            prompt = VoiceSend.Prompt(startedAt: Date())
            speech.stop()
            startRecognition(.listening)
        }
    }

    /// Mic off, tidy the text up on the Mac, then send: at once after "send", after a short Undo after yes.
    /// If the cleanup fails or the Mac can't be reached, the dictation goes as it was.
    private func submit(text message: String, withUndo: Bool) {
        let original = message.trimmingCharacters(in: .whitespacesAndNewlines)
        silenceWatch?.cancel()
        speech.stop()
        // Answered while the question was still being read out.
        if speaker.isSpeaking && !speaker.interruptible { speaker.stop() }
        text = original
        guard !original.isEmpty else { phase = .idle; return }
        guard store.canWrite, !holdAutoSend() else {
            VoiceLog.shared.add("send held: \(store.canWrite ? "a “did you mean” question is up" : "offline"); kept")
            phase = .idle
            return
        }
        phase = .cleaning
        Task {
            let (cleaned, how) = await store.cleanDictation(original)
            guard phase == .cleaning else { return }
            VoiceLog.shared.add("cleanup: \(how)")
            text = cleaned
            phase = .idle
            voiceSending = true
            if withUndo {
                VoiceLog.shared.add("sending in \(Int(VoiceSend.undoWindow)) s with Undo")
                scheduleSend(cleaned, in: VoiceSend.undoWindow)
            } else {
                sendNow()
            }
        }
    }

    /// Hands the send to the store. On screen when it fires, it's a Send; after you left, only the message goes
    /// (no reading the reply out or listening again).
    private func scheduleSend(_ message: String, in window: TimeInterval) {
        let shown = shown, field = $text, send = send
        store.scheduleVoiceSend(message, to: agentId, in: window) {
            if shown.on { sendNow() } else { field.wrappedValue = ""; send(message, true) }
        }
    }

    private func sendNow() {
        if pending != nil { store.cancelVoiceSend() }
        if micOn {
            // Tapping Send mid-dictation sends what's there.
            silenceWatch?.cancel()
            phase = .idle
            speech.stop()
        }
        let byVoice = voiceSending || dictated
        voiceSending = false
        dictated = false
        let t = trimmed
        guard !t.isEmpty || !attachments.wrappedValue.isEmpty, !attachmentsHold, store.canWrite else {
            VoiceLog.shared.add("send blocked: \(t.isEmpty ? "empty" : "offline")")
            return
        }
        VoiceLog.shared.add("sent \(VoiceLog.clip(t))")
        text = ""
        send(t, byVoice)
        if byVoice { onVoiceSent?() }
    }

    private func remove(_ draft: AttachmentDraft) {
        draft.cancel()
        attachments.wrappedValue.removeAll { $0 === draft }
    }

    /// Undo on the Sending bar: nothing goes out, the (cleaned) text stays to edit.
    private func undoSend() {
        VoiceLog.shared.add("Undo tapped: not sent, kept for editing")
        cancelAutoSend()
    }

    private func cancelAutoSend() {
        if pending != nil { store.cancelVoiceSend() }
        voiceSending = false
        if phase == .cleaning { phase = .idle }
    }
}

private final class Shown { var on = false }

/// "Would you like to submit this?" with Yes / No, while the mic listens for the answer.
struct SubmitPromptBar: View {
    @Environment(\.dynamicTypeSize) private var typeSize
    let question: String
    let yes: () -> Void
    let no: () -> Void

    var body: some View {
        let ax = typeSize.isAccessibilitySize
        VStack(alignment: .leading, spacing: 6) {
            Label(question, systemImage: "questionmark.bubble")
                .font(.subheadline.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            Text("Say yes or no, or keep talking.")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            (ax ? AnyLayout(VStackLayout(spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))) {
                if !ax { Spacer(minLength: 0) }
                Button(action: no) { Text("No, let me edit").multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true).frame(maxWidth: ax ? .infinity : nil) }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("submitPromptNo")
                Button(action: yes) { Text("Yes, send").multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true).frame(maxWidth: ax ? .infinity : nil) }
                    .buttonStyle(.accent)
                    .accessibilityIdentifier("submitPromptYes")
            }
        }
        .padding(10)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("submitPrompt")
    }
}

/// What the chat can ask its composer to do.
enum ComposerControl: Equatable {
    case listen
    case cancelDictation
    /// Mic off and any countdown stopped, keeping the words (e.g. the interview ended).
    case stopListening
}

/// "Sending to Orchestrator… Undo · Send now", with a bar that runs out when it goes.
struct AutoSendBar: View {
    @Environment(\.dynamicTypeSize) private var typeSize
    let recipient: String
    /// When it goes; the bar runs out then.
    let deadline: Date
    let undo: () -> Void
    let sendNow: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            // The label on its own line and the buttons under it: side by side, both wrapped onto two lines.
            // A word joiner before the ellipsis keeps it on the name's line when the label wraps.
            // At accessibility sizes the ellipsis ended up alone on a third line.
            Label(typeSize.isAccessibilitySize ? "Sending to \(recipient)" : "Sending to \(recipient)\u{2060}…", systemImage: "paperplane")
                .font(.subheadline.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
            let ax = typeSize.isAccessibilitySize
            (ax ? AnyLayout(VStackLayout(spacing: 8)) : AnyLayout(HStackLayout(spacing: 8))) {
                if !ax { Spacer(minLength: 0) }
                Button(action: undo) { Text("Undo").frame(maxWidth: ax ? .infinity : nil) }
                    .buttonStyle(.bordered)
                    .accessibilityHint("Doesn't send; keeps the message so you can edit it")
                    .accessibilityIdentifier("autoSendUndo")
                Button(action: sendNow) { Text("Send now").fixedSize().frame(maxWidth: ax ? .infinity : nil) }
                    .buttonStyle(.accent)
                    .accessibilityIdentifier("autoSendNow")
            }
            .controlSize(.regular)
            // Paused under UI tests: a bar redrawn ten times a second keeps the app from ever going idle.
            TimelineView(.animation(minimumInterval: 0.1, paused: TestMode.noAnimations)) { context in
                ProgressView(value: min(1, max(0, 1 - deadline.timeIntervalSince(context.date) / VoiceSend.undoWindow)))
                    .tint(Theme.accent)
                    .accessibilityHidden(true)
            }
            .accessibilityHidden(true)
        }
        .padding(10)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .onAppear {
            UIAccessibility.post(notification: .announcement, argument: "Sending to \(recipient). Tap Undo to keep it.")
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("autoSendBar")
    }
}

/// The agents to mention while "@…" is typed: sticker and name. Picking one puts its exact name in the message.
struct MentionPicker: View {
    let sections: [(title: String, agents: [Agent])]
    let pick: (Agent) -> Void

    var body: some View {
        let rows = sections.reduce(0) { $0 + $1.agents.count + 1 }
        if sections.isEmpty {
            Text("No agent by that name").font(.footnote).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            VStack(alignment: .leading, spacing: 0) {
                Text("Also sends the message to them, and their reply shows here.")
                    .font(.caption).foregroundStyle(.secondary).padding(.bottom, 4)
                // Every match, scrolling past about five rows so the chat above stays in view.
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(sections, id: \.title) { section in
                            Text(section.title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                                .padding(.top, 6)
                                .accessibilityAddTraits(.isHeader)
                                .accessibilityIdentifier("mentionSection-\(section.title)")
                            ForEach(section.agents) { a in
                                Button { pick(a) } label: {
                                    HStack(spacing: 10) {
                                        AgentAvatar(name: a.name, id: a.id, size: 28)
                                        Text(a.name).font(.subheadline).foregroundStyle(.primary)
                                        Spacer()
                                    }
                                    .frame(minHeight: 44)
                                    .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel("Mention \(a.name)")
                                .accessibilityIdentifier("mention-\(a.name)")
                            }
                        }
                    }
                }
                .frame(maxHeight: 44 * 5.5)
                .fixedSize(horizontal: false, vertical: rows <= 6)
                .scrollBounceBehavior(.basedOnSize)
                .accessibilityIdentifier("mentionList")
            }
            .padding(10)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Mention an agent")
        }
    }
}

/// The specs, issues and pull requests to tag while "#…" is typed. Picking one puts its tag in the message.
struct TagPicker: View {
    let rows: [TagQuery.Row]
    let pick: (TagQuery.Row) -> Void

    private func symbol(_ k: TagQuery.Kind) -> String {
        switch k { case .spec: "doc.text"; case .issue: "circle.dotted"; case .pr: "arrow.triangle.pull" }
    }

    var body: some View {
        // No match shows nothing, like the web menu: "#fff" in a message is often just text.
        if !rows.isEmpty {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(rows) { r in
                        Button { pick(r) } label: {
                            HStack(spacing: 10) {
                                Image(systemName: symbol(r.kind)).foregroundStyle(r.open ? .secondary : .tertiary).frame(width: 22)
                                Text(r.title).font(.subheadline).foregroundStyle(.primary).lineLimit(1)
                                Spacer(minLength: 8)
                                Text(r.number.map { "#\($0)" } ?? r.kind.rawValue).font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                            .frame(minHeight: 44)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Tag \(r.kind.rawValue): \(r.title)")
                        .accessibilityIdentifier("tag-\(r.title)")
                    }
                }
            }
            .frame(maxHeight: 44 * 5.5)
            .fixedSize(horizontal: false, vertical: rows.count <= 6)
            .scrollBounceBehavior(.basedOnSize)
            .padding(10)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Tag a spec, issue or pull request")
            .accessibilityIdentifier("tagList")
        }
    }
}

/// A `#` tag on a sent message: kind and title, opening the Loft issue or the pull request on GitHub.
struct TagChip: View {
    let tag: MessageTag

    var body: some View {
        let label = HStack(spacing: 4) {
            Text(tag.kind).foregroundStyle(.tertiary)
            Text(tag.title).lineLimit(1)
        }
        .font(.caption2.weight(.semibold))
        .padding(.horizontal, 8).padding(.vertical, 4)
        .background(Color.secondary.opacity(0.12), in: Capsule())
        .foregroundStyle(.secondary)
        if let id = tag.taskId {
            NavigationLink(value: TaskRoute(id: id)) { label }.buttonStyle(.plain)
                .accessibilityLabel("\(tag.kind): \(tag.title)").accessibilityHint("Opens it")
        } else if let url = tag.url.flatMap(URL.init(string:)) {
            Link(destination: url) { label }
                .accessibilityLabel("\(tag.kind): \(tag.title)").accessibilityHint("Opens it on GitHub")
        } else {
            label
        }
    }
}
