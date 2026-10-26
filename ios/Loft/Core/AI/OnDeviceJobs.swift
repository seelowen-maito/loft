import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

// The jobs Loft gives Apple's on-device model, kept free of app types so the evaluation runner
// (ios/Evaluations, on the Mac) runs exactly the same instructions and output types as the app.
// Each job: short fixed instructions, a small prompt, typed output, one fresh session per call.

#if canImport(FoundationModels)
@available(iOS 26.0, macOS 26.0, *)
enum OnDeviceJobs {
    /// A short spoken reply to a yes/no question ("Would you like to submit this?").
    @Generable
    struct YesNo: Equatable {
        @Guide(description: "neither unless the reply is only a yes or only a no")
        var verdict: Verdict
    }

    @Generable
    enum Verdict: String, Equatable {
        case neither, yes, no
    }

    static let yesNoInstructions = """
        Someone dictated a message and was asked whether to send it. Classify their spoken reply. \
        It may be English or Portuguese, with recognition errors.
        - yes: only agreeing to send, nothing more. "yes", "sure, send it", "go ahead", "sim", "pode mandar".
        - no: only declining or asking to wait or check first. "no", "not yet", "hold on", "wait, let me check", "não", "espera".
        - neither: anything that adds to the message or says something else, even if it starts with yes or no. \
        "yes and add the tests too", "the site is on staging", "no idea, ask the dev", "e também atualiza o README".
        When unsure, answer neither.
        """

    static func yesNo(_ reply: String, question: String) async throws -> Verdict {
        let session = LanguageModelSession(instructions: yesNoInstructions)
        let r = try await session.respond(to: "Question: \(question)\nReply: \(reply)", generating: YesNo.self,
                                          options: GenerationOptions(temperature: 0))
        return r.content.verdict
    }

    /// What was said while an interview question waits: an answer, or a command for the interview.
    @Generable
    struct GrillReading: Equatable {
        // Generated first, so the intent is chosen after deciding who the reply is about.
        @Guide(description: "true only if the person is talking about the interview itself: moving to another question, pausing, continuing, stopping, recapping, hearing the question again, or letting the interviewer choose")
        var aboutTheInterview: Bool
        @Guide(description: "answer unless aboutTheInterview is true")
        var intent: GrillIntent
    }

    @Generable
    enum GrillIntent: String, Equatable {
        case answer, skip, back, pause, resume, end, summarize, decide, `repeat`
    }

    static let grillInstructions = """
        An interviewer asked a question and the person replied by voice (English or Portuguese, maybe with \
        recognition errors). Decide whether the reply answers the question or controls the interview.
        - skip: move on without answering ("let's move on", "next one please", "pula essa").
        - back: return to the previous question ("go back to the last one", "volta uma").
        - pause / resume: stop for now / carry on ("let's take a break", "vamos continuar").
        - end: finish the whole interview ("let's stop here", "we're done with this", "chega por hoje").
        - summarize: recap what was covered ("where are we so far?", "faz um resumo").
        - decide: the person has no opinion and lets the interviewer choose ("you pick", "whatever you think", "tanto faz").
        - repeat: hear the question again ("what was the question?", "can you say that again?", "não entendi").
        - answer: everything else, including answers that start with or mention those words \
        ("skip the login screen for now", "next month, probably", "stop sending emails on weekends", \
        "I don't know if it scales, let's test it", "volta pro design antigo"). When unsure, answer.
        """

    static func grill(_ said: String, question: String) async throws -> GrillIntent {
        let session = LanguageModelSession(instructions: grillInstructions)
        let r = try await session.respond(to: "Question: \(question)\nReply: \(said)", generating: GrillReading.self,
                                          options: GenerationOptions(temperature: 0))
        // Belt and braces: unless it's about the interview itself, it's an answer, whatever intent came with it.
        return r.content.aboutTheInterview ? r.content.intent : .answer
    }

    /// A long agent reply, said in a few plain sentences for reading aloud.
    @Generable
    struct Spoken: Equatable {
        @Guide(description: "the main outcome in one short sentence of at most 20 words")
        var outcome: String
        @Guide(description: "what the person needs to do or decide, in one short sentence of at most 15 words; empty if nothing")
        var forYou: String

        var text: String { [outcome, forYou].map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }.joined(separator: " ") }
    }

    static let spokenInstructions = """
        You turn a software agent's written reply into what a colleague would say out loud in a few seconds. \
        Give the main outcome (what was done or what failed) and, if there is one, what the person needs to do or \
        decide. Skip details, counts and who else is working on it. Leave out PR and issue numbers, links, file paths, code, commit hashes and \
        lists. Don't add anything that isn't in the reply. Write in the same language as the reply.
        """

    /// `language`: "pt", "en", … (the reply's, from the app's language detection).
    static func spoken(_ reply: String, language: String?) async throws -> String {
        let session = LanguageModelSession(instructions: spokenInstructions)
        let name = language.flatMap { Locale(identifier: "en").localizedString(forLanguageCode: $0) } ?? "the reply's language"
        let prompt = """
            Say this in \(name), in your own words (don't copy its sentences):

            \(spokenInput(reply))
            """
        let r = try await session.respond(to: prompt, generating: Spoken.self, options: GenerationOptions(temperature: 0))
        return r.content.text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The reply without what can't be said: code, links, PR and issue numbers, Markdown.
    static func spokenInput(_ reply: String) -> String {
        var s = reply
        s = s.replacingOccurrences(of: #"```[\s\S]*?```"#, with: "", options: .regularExpression)
        s = s.replacingOccurrences(of: #"`[^`]*`"#, with: "", options: .regularExpression)
        s = s.replacingOccurrences(of: #"\[([^\]]+)\]\([^)]+\)"#, with: "$1", options: .regularExpression)
        s = s.replacingOccurrences(of: #"https?://\S+"#, with: "", options: .regularExpression)
        s = s.replacingOccurrences(of: #"#\d+"#, with: "", options: .regularExpression)
        s = s.replacingOccurrences(of: #"[*_]+"#, with: "", options: .regularExpression)
        return s
    }

    /// What makes a spoken version unusable; nil when it's fine. Shared by the app (to fall back) and the evaluation.
    static func problem(withSpoken s: String, for reply: String) -> String? {
        if s.isEmpty { return "empty" }
        if s.count > 360 { return "too long (\(s.count) chars)" }
        if s.range(of: #"https?://|#\d+|`|/[\w.-]+/|\b[0-9a-f]{7,40}\b"#, options: .regularExpression) != nil { return "has a link, number, path, code or hash" }
        let sentences = s.split(whereSeparator: { ".!?".contains($0) }).filter { $0.trimmingCharacters(in: .whitespaces).count > 2 }
        if sentences.count > 4 { return "\(sentences.count) sentences" }
        return nil
    }

    /// A first reply in under a second, while the Orchestrator works on the real one; and who the message is for.
    /// The model only fills small slots; the app writes the sentence ("Got it, looking into …"), so it can't
    /// claim anything happened, and maps project + role to an agent, so it can't invent one.
    @Generable
    struct QuickRead: Equatable {
        @Guide(description: "3 to 8 words copied from the message that say what it is about, in the message's own language")
        var subject: String
    }

    /// Who a message is for: a project (one of the agents' own, or "unclear") and the kind of work.
    struct Route: Equatable {
        static let unclear = "unclear"
        var project: String
        var role: Role
    }

    enum Role: String, Equatable, CaseIterable {
        case backend, iphone, testing, security, marketing, merging, overview
    }

    static let quickReadInstructions = """
        Read a message someone sent to their assistant. Copy the few words from it that say what it is about \
        (3 to 8 words, in the message's language, not translated or rephrased).
        """

    /// Words that name a role rather than a project, so "QA Agent" doesn't make a project called "qa".
    static let roleWords: Set<String> = ["dev", "developer", "backend", "frontend", "web", "mobile", "ios", "iphone", "qa", "tester",
                                         "test", "security", "marketing", "merger", "agent", "pr", "orchestrator"]

    /// The projects routing can choose from: each agent's project, or the first word of a name like "Shop Dev".
    static func projects(of agents: [RouteTarget]) -> [String] {
        var seen: [String] = []
        for a in agents {
            var p = VoiceText.normalize(a.project ?? "")
            if p.isEmpty {
                let words = VoiceText.normalize(a.name).split(separator: " ").map(String.init)
                if words.count > 1, !roleWords.contains(words[0]) { p = words[0] }
            }
            if !p.isEmpty, p != Route.unclear, !seen.contains(p) { seen.append(p) }
        }
        return seen
    }

    static func routeInstructions(projects: [String]) -> String {
        let list = (projects.map { $0 == "loft" ? "loft (this app: the Mac server, web dashboard, iPhone app, voice, agents)" : $0 }
                    + [Route.unclear]).joined(separator: ", ")
        return """
            Read a message someone sent to their Orchestrator, which coordinates coding agents, and say who should do it.
            project: the project it is about, one of: \(list).
            role: backend (server, web, endpoints, data, fixes outside the iPhone app), iphone (the iPhone app, its screens, \
            voice and microphone in the app), testing (run tests, try a build, QA), security (audits, secrets, leaks), marketing \
            (posts, launches), merging (merge PRs), or overview (status, plans, summaries, approvals, anything across agents \
            or projects).
            """
    }

    /// The route's output schema, built from the projects that exist, so the model can only pick one of them.
    static func routeSchema(projects: [String]) throws -> GenerationSchema {
        let project = DynamicGenerationSchema(name: "Project", description: "the project the message is about",
                                              anyOf: projects + [Route.unclear])
        let role = DynamicGenerationSchema(name: "Role", description: "the kind of work it asks for",
                                           anyOf: Role.allCases.map(\.rawValue))
        let route = DynamicGenerationSchema(name: "Route", properties: [
            .init(name: "project", schema: project), .init(name: "role", schema: role),
        ])
        return try GenerationSchema(root: route, dependencies: [])
    }

    static func quickRead(_ message: String) async throws -> QuickRead {
        let session = LanguageModelSession(instructions: quickReadInstructions)
        let r = try await session.respond(to: spokenInput(message), generating: QuickRead.self,
                                          options: GenerationOptions(temperature: 0))
        return r.content
    }

    /// Who `message` is for, among the projects of `agents`.
    static func route(_ message: String, agents: [RouteTarget]) async throws -> Route {
        let projects = projects(of: agents)
        let session = LanguageModelSession(instructions: routeInstructions(projects: projects))
        let r = try await session.respond(to: spokenInput(message), schema: try routeSchema(projects: projects),
                                          options: GenerationOptions(temperature: 0))
        let project = try r.content.value(String.self, forProperty: "project")
        let role = Role(rawValue: try r.content.value(String.self, forProperty: "role")) ?? .overview
        var route = Route(project: projects.contains(project) ? project : Route.unclear, role: role)
        let words = Set(VoiceText.normalize(message).split(separator: " ").map(String.init))
        // About everyone, or a plan across agents: the Orchestrator's own job, whatever the model says.
        let together = ["everyone", "everybody", "all", "agents", "anyone", "team", "mundo", "ninguem", "equipe"]
        let planning = ["plan", "split", "summarize", "summary", "status", "recap", "planeja", "divide", "resume", "resumo"]
        if words.contains(where: together.contains) || words.contains(where: planning.contains) {
            route.role = .overview
            return route
        }
        // A project named in the message wins over the model's guess.
        for project in projects where words.contains(project) {
            route.project = project
        }
        return route
    }

    /// "Got it, looking into …" in the message's language.
    static func ack(subject: String, language: String?) -> String {
        let s = subject.trimmingCharacters(in: CharacterSet(charactersIn: " .!?\n"))
        switch language {
        case "pt": return "Entendi, vou ver \(s)."
        default: return "Got it, looking into \(s)."
        }
    }

    /// An agent as routing sees it: its name and project ("Shop Dev", "shop").
    struct RouteTarget: Equatable {
        var name: String
        var project: String?
    }

    /// The agent for a project and role, from the agents that exist: in that project (by its project or its name's first
    /// word), and named for the role ("… Dev", "… mobile dev" / "iOS …", "… Tester" / "… QA", …). Nil for the Orchestrator.
    static func agent(for r: Route, agents: [RouteTarget]) -> String? {
        guard r.project != Route.unclear, r.role != .overview else { return nil }
        let project = r.project
        let mine = agents.filter { a in
            VoiceText.normalize(a.project ?? "") == project || VoiceText.normalize(a.name).hasPrefix(project + " ")
        }
        func named(_ any: [[String]], not excluded: [String] = []) -> String? {
            for words in any {
                if let a = mine.first(where: { a in
                    let n = Set(VoiceText.normalize(a.name).split(separator: " ").map(String.init))
                    return words.allSatisfy(n.contains) && excluded.allSatisfy { !n.contains($0) }
                }) { return a.name }
            }
            return nil
        }
        switch r.role {
        case .backend: return named([["dev"], ["backend"]], not: ["mobile", "ios"])
        case .iphone: return named([["mobile", "dev"], ["ios"]]) ?? named([["dev"]])
        case .testing: return named([["tester"], ["qa"]])
        case .security: return named([["security"]])
        case .marketing: return named([["marketing"]])
        case .merging: return named([["merger"]])
        case .overview: return nil
        }
    }

    /// Words that would claim something happened; an acknowledgement only says what will be looked at.
    static let claims = ["done", "fixed", "merged", "deployed", "finished", "completed", "approved", "sent", "solved",
                         "pronto", "feito", "corrigi", "corrigido", "enviei", "enviado", "aprovado", "resolvido", "conclu"]

    /// What makes an acknowledgement's subject unusable for `message`; nil when it's fine. The subject must come
    /// from the message (so it's faithful and in its language), and add no claim the message didn't make.
    static func problem(withSubject subject: String, message: String) -> String? {
        let said = Set(VoiceText.normalize(message).split(separator: " ").map(String.init))
        let words = VoiceText.normalize(subject).split(separator: " ").map(String.init)
        guard words.count >= 2 else { return "too short" }
        let fromMessage = words.filter { w in said.contains(w) || said.contains { $0.hasPrefix(w) || w.hasPrefix($0) } }
        if Double(fromMessage.count) / Double(words.count) < 0.8 { return "not from the message" }
        if let c = claims.first(where: { c in words.contains { $0.hasPrefix(c) } && !said.contains { $0.hasPrefix(c) } }) {
            return "claims something happened (\(c))"
        }
        return problem(withAck: subject)
    }

    /// What makes an acknowledgement unusable; nil when it's fine.
    static func problem(withAck s: String) -> String? {
        if s.isEmpty { return "empty" }
        if s.count > 110 { return "too long (\(s.count) chars)" }
        if s.range(of: #"https?://|#\d+|`|\d{2,}"#, options: .regularExpression) != nil { return "has a link, number or code" }
        let sentences = s.split(whereSeparator: { ".!?".contains($0) }).filter { $0.trimmingCharacters(in: .whitespaces).count > 2 }
        if sentences.count > 2 { return "\(sentences.count) sentences" }
        return nil
    }


    /// A "Needs you" item in one line, and two or three short replies the person might tap (they fill the answer box;
    /// nothing is sent until they press Send).
    @Generable
    struct Assist: Equatable {
        @Guide(description: "what the agent needs from the person, in one short sentence of at most 15 words, in the text's language")
        var summary: String
        @Guide(description: "two or three short replies the person might give, each at most 6 words, in the text's language; empty if the text asks nothing", .maximumCount(3))
        var replies: [String]
    }

    static let assistInstructions = """
        A coding agent is waiting on a person. Read what it wrote and fill in:
        summary: what it needs from the person (a choice, an answer, an approval), in one plain sentence. \
        No numbers, links, file names or code.
        replies: two or three short answers the person might give. When the agent offers options, one reply per option, \
        using the option's own words. Otherwise simple ones such as yes, no or later. Never more than 6 words each.
        Write in the same language as the agent's text.
        """

    static func assist(title: String, text: String, language: String?) async throws -> Assist {
        let session = LanguageModelSession(instructions: assistInstructions)
        let name = language.flatMap { Locale(identifier: "en").localizedString(forLanguageCode: $0) } ?? "the text's language"
        let r = try await session.respond(to: "Answer in \(name).\nTitle: \(title)\nText: \(spokenInput(text))", generating: Assist.self,
                                          options: GenerationOptions(temperature: 0))
        return r.content
    }

    /// What makes a summary unusable; nil when it's fine. Most of its words have to come from the item. Bad replies
    /// are dropped one by one by `usableReplies`.
    static func problem(withSummary s: String, for text: String) -> String? {
        let said = Set(VoiceText.normalize(text).split(separator: " ").map(String.init))
        let words = VoiceText.normalize(s).split(separator: " ").map(String.init).filter { $0.count > 3 }
        let grounded = words.filter { w in said.contains(w) || said.contains { $0.hasPrefix(String(w.prefix(5))) } }
        if !words.isEmpty, Double(grounded.count) / Double(words.count) < 0.5 { return "not from the item" }
        if s.trimmingCharacters(in: .whitespaces).count < 8 { return "too short" }
        if s.count > 140 { return "too long (\(s.count) chars)" }
        if s.range(of: #"https?://|#\d+|`"#, options: .regularExpression) != nil { return "has a link, number or code" }
        return nil
    }

    /// Words a reply may use without them being in the agent's text.
    static let replyWords: Set<String> = ["yes", "no", "not", "now", "later", "go", "ahead", "ok", "please", "do", "it", "use",
                                          "the", "a", "an", "both", "neither", "first", "wait", "skip", "sure", "thanks",
                                          "sim", "nao", "agora", "depois", "pode", "seguir", "usa", "use", "o", "a", "os", "as",
                                          "primeiro", "espera", "pular", "ambos", "nenhum", "obrigado", "faz", "isso", "claro"]

    /// The replies worth offering for `text`: short, distinct, and only using words from it (or plain ones like
    /// "yes" and "later"), so a reply can't offer an option the agent never gave.
    static func usableReplies(_ replies: [String], for text: String) -> [String] {
        var seen = Set<String>()
        let said = Set(VoiceText.normalize(text).split(separator: " ").map(String.init))
        return replies.map { $0.trimmingCharacters(in: CharacterSet(charactersIn: " .\"“”")) }
            .filter { r in
                let n = VoiceText.normalize(r)
                let words = n.split(separator: " ").map(String.init)
                let grounded = words.allSatisfy { w in replyWords.contains(w) || said.contains(w) || said.contains { $0.hasPrefix(w) || w.hasPrefix($0) } }
                guard !n.isEmpty, words.count <= 6, r.count <= 40, grounded, !seen.contains(n) else { return false }
                seen.insert(n)
                return true
            }
            .prefix(3).map { $0 }
    }

    /// Jobs by name, for the evaluation runner: (input, context) → the label to compare with the expected one.
    /// A permission request in plain words: what the agent's tool call would do. Risks are decided in code
    /// (PermissionRisk); `PermissionExplainer.problem` drops a sentence that sounds reassuring.
    @Generable
    struct Explanation: Equatable {
        @Guide(description: "what allowing this would do, in one plain sentence of at most 25 words")
        var what: String
    }

    static let explainInstructions = """
        A coding agent on a Mac asks permission to run a tool. Say in one plain sentence what allowing it would \
        do, for someone who doesn't read code: what it reads, what it changes or deletes, and what it sends \
        elsewhere. Only what this command does; if it only shows or lists something, say so. \
        Don't judge whether it's safe. No code or quoting.
        """

    static func explain(tool: String, request: String, reason: String) async throws -> Explanation {
        let session = LanguageModelSession(instructions: explainInstructions)
        var prompt = "Tool: \(tool)\nRequest: \(request)"
        if !reason.isEmpty { prompt += "\nThe agent's reason: \(reason)" }
        let r = try await session.respond(to: prompt, generating: Explanation.self, options: GenerationOptions(temperature: 0))
        return r.content
    }

    static let registry: [String: @Sendable (String, String) async throws -> String] = [
        "yes_no": { input, context in try await yesNo(input, question: context).rawValue },
        "grill": { input, context in try await grill(input, question: context).rawValue },
    ]
}
#endif
