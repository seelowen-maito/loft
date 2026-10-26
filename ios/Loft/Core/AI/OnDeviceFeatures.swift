import Foundation

/// The app's uses of the on-device model, each behind its Settings switch, and always with today's
/// behaviour as the fallback (OnDeviceModel).
enum OnDeviceFeatures {
    /// Settings › Apple Intelligence › Understand voice commands, and the model can run.
    static var commandsOn: Bool {
        UserDefaults.standard.bool(forKey: Prefs.aiCommands) && OnDeviceModel.status.isAvailable
    }

    /// The model's reading of a short interview reply (answer, skip, back, …), or nil when it can't give one in time.
    static func grillLabel(_ text: String, question: String) async -> String? {
        #if canImport(FoundationModels)
        guard commandsOn, #available(iOS 26.0, *) else { return nil }
        return await OnDeviceModel.run("interview reply", fallback: { nil }) {
            try await OnDeviceJobs.grill(text, question: question).rawValue
        }.value
        #else
        return nil
        #endif
    }

    static var shortSpokenOn: Bool {
        UserDefaults.standard.bool(forKey: Prefs.aiShortSpoken) && OnDeviceModel.status.isAvailable
    }

    /// A long reply said in a few plain sentences, or nil to use today's short read (its opening).
    static func spokenVersion(of reply: String) async -> String? {
        #if canImport(FoundationModels)
        guard shortSpokenOn, #available(iOS 26.0, *) else { return nil }
        let language = VoiceText.dominantLanguage(reply)
        let out = await OnDeviceModel.run("short spoken version", timeout: .seconds(4), fallback: { String?.none }) {
            try await OnDeviceJobs.spoken(reply, language: language)
        }.value
        guard let out else { return nil }
        if let p = OnDeviceJobs.problem(withSpoken: out, for: reply) {
            Task { @MainActor in VoiceLog.shared.add("ai: short spoken version not used (\(p))") }
            return nil
        }
        return out
        #else
        return nil
        #endif
    }

    /// A quick acknowledgement has to be quicker than this, or the usual "Got it, thinking…" stays.
    static let quickTimeout: Duration = .milliseconds(1500)

    /// "Got it, looking into …" for a message just sent to the Orchestrator, or nil (then "Got it, thinking…").
    static func quickAck(for message: String) async -> String? {
        #if canImport(FoundationModels)
        guard UserDefaults.standard.bool(forKey: Prefs.aiQuickAck), OnDeviceModel.status.isAvailable, #available(iOS 26.0, *) else { return nil }
        let subject = await OnDeviceModel.run("quick acknowledgement", timeout: quickTimeout, fallback: { String?.none }) {
            try await OnDeviceJobs.quickRead(message).subject
        }.value
        guard let subject else { return nil }
        if let p = OnDeviceJobs.problem(withSubject: subject, message: message) {
            Task { @MainActor in VoiceLog.shared.add("ai: quick acknowledgement not used (\(p))") }
            return nil
        }
        return OnDeviceJobs.ack(subject: subject, language: VoiceText.dominantLanguage(message))
        #else
        return nil
        #endif
    }


    #if canImport(FoundationModels)
    @available(iOS 26.0, *)
    static func routeGuess(for message: String, agents: [Agent], orchestratorId: String?) async -> Agent? {
        guard UserDefaults.standard.bool(forKey: Prefs.aiRoute), OnDeviceModel.status.isAvailable else { return nil }
        let targets = agents.filter { $0.id != orchestratorId }.map { OnDeviceJobs.RouteTarget(name: $0.name, project: $0.project) }
        let name = await OnDeviceModel.run("route guess", timeout: quickTimeout, fallback: { String?.none }) {
            OnDeviceJobs.agent(for: try await OnDeviceJobs.route(message, agents: targets), agents: targets)
        }.value
        return name.flatMap { n in agents.first { $0.name == n } }
    }
    #endif

    /// A one-line summary and up to three suggested replies for a "Needs you" item, or nil (then its reason, no chips).
    static func needsYouAssist(_ item: NeedsYouItem) async -> NeedsYouAssist? {
        #if canImport(FoundationModels)
        guard UserDefaults.standard.bool(forKey: Prefs.aiNeedsYou), OnDeviceModel.status.isAvailable, #available(iOS 26.0, *) else { return nil }
        let language = VoiceText.dominantLanguage(item.text)
        let a = await OnDeviceModel.run("needs-you \(item.type)", timeout: .seconds(4), fallback: { OnDeviceJobs.Assist?.none }) {
            try await OnDeviceJobs.assist(title: item.title, text: item.text, language: language)
        }.value
        guard let a else { return nil }
        let summary = OnDeviceJobs.problem(withSummary: a.summary, for: item.title + " " + item.text) == nil ? a.summary : nil
        let replies = item.answerAction == nil ? [] : OnDeviceJobs.usableReplies(a.replies, for: item.text)
        if summary == nil, replies.isEmpty { return nil }
        return NeedsYouAssist(summary: summary, replies: replies)
        #else
        return nil
        #endif
    }

    /// Two or three one-tap replies to an agent's message that ends on a question, or none. Measured on 40 chat
    /// messages (ios/Evaluations/cases/chat_replies.json): 82%, and the misses show no replies rather than wrong ones.
    static func chatReplies(to text: String) async -> [String] {
        #if DEBUG
        // `-fixture-chat-replies "Merge it now|Wait for QA"`: these instead of the model (UI tests, screenshots).
        if let f = FixtureAPI.launchText("fixture-chat-replies") {
            return ChatReplies.asksSomething(text) ? f.split(separator: "|").map(String.init) : []
        }
        #endif
        #if canImport(FoundationModels)
        guard UserDefaults.standard.bool(forKey: Prefs.aiChatReplies), OnDeviceModel.status.isAvailable,
              ChatReplies.asksSomething(text), #available(iOS 26.0, *) else { return [] }
        let a = await OnDeviceModel.run("chat replies", timeout: .seconds(4), fallback: { OnDeviceJobs.Assist?.none }) {
            try await OnDeviceJobs.assist(title: "The agent asked you something", text: text, language: VoiceText.dominantLanguage(text))
        }.value
        guard let a else { return [] }
        return ChatReplies.shown(OnDeviceJobs.usableReplies(a.replies, for: text), for: text)
        #else
        return []
        #endif
    }

    static var explainOn: Bool {
        #if DEBUG
        if UserDefaults.standard.string(forKey: "fixture-explain") != nil { return true }
        #endif
        return UserDefaults.standard.bool(forKey: Prefs.aiExplain) && OnDeviceModel.status.isAvailable
    }

    /// A permission request in one plain sentence, or nil (the card shows the request as it is). On request only.
    static func explain(_ r: PermissionRequest) async -> String? {
        #if DEBUG
        if let f = UserDefaults.standard.string(forKey: "fixture-explain") { return f }
        #endif
        #if canImport(FoundationModels)
        guard explainOn, #available(iOS 26.0, *) else { return nil }
        let request = r.detail
        let e = await OnDeviceModel.run("permission explainer", timeout: .seconds(5), fallback: { OnDeviceJobs.Explanation?.none }) {
            try await OnDeviceJobs.explain(tool: r.toolName, request: request, reason: r.reason ?? "")
        }.value
        guard let e else { return nil }
        if let p = PermissionExplainer.problem(e.what, request: r.toolName + " " + request, risks: r.risks) {
            await MainActor.run { VoiceLog.shared.add("ai: permission explanation not used (\(p))") }
            return nil
        }
        return PermissionExplainer.cleaned(e.what, tool: r.toolName)
        #else
        return nil
        #endif
    }

    /// Replies this short that the phrase lists don't know ("sounds good", "do it") go to the model.
    static let maxWordsForSubmitAnswer = 3

    /// The answer to "Would you like to submit this?": the phrase list first, the model for very short replies it
    /// doesn't know. Nil means it's more of the message.
    static func submitAnswer(_ text: String, question: String) async -> VoiceSend.Answer? {
        if let a = VoiceSend.answer(text) { return a }
        #if canImport(FoundationModels)
        guard commandsOn, #available(iOS 26.0, *),
              (1...maxWordsForSubmitAnswer).contains(VoiceText.normalize(text).split(separator: " ").count) else { return nil }
        let verdict = await OnDeviceModel.run("submit answer", fallback: { OnDeviceJobs.Verdict.neither }) {
            try await OnDeviceJobs.yesNo(text, question: question)
        }.value
        switch verdict {
        case .yes: return .yes
        case .no: return .no
        case .neither: return nil
        }
        #else
        return nil
        #endif
    }
}

struct NeedsYouAssist: Equatable {
    var summary: String?
    var replies: [String]
}

