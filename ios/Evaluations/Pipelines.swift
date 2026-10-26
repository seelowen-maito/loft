import Foundation

/// The app's full readings (phrase matcher first, then the model), as the evaluation measures them, with the
/// server's phrase list as the app gets it (`grill_phrases.json`, from `GET /api/grill/commands`).
enum Pipelines {
    static let phrases: GrillPhrases? = {
        let url = URL(fileURLWithPath: "grill_phrases.json")
        guard let data = try? Data(contentsOf: url) else { return nil }
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return try? d.decode(GrillPhrases.self, from: data)
    }()

    /// Your own agents, for the route set: `local/agents.json` (gitignored), a list of
    /// `{"name": "Shop Dev", "project": "shop"}` (project optional), as the phone has them.
    static let agents: [OnDeviceJobs.RouteTarget] = {
        struct A: Decodable { let name: String; let project: String? }
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: "local/agents.json")),
              let list = try? JSONDecoder().decode([A].self, from: data) else { return [] }
        return list.filter { $0.name != "Orchestrator" }.map { .init(name: $0.name, project: $0.project) }
    }()

    static let registry: [String: @Sendable (String, String) async throws -> String] = [
        // A permission request explained: "ok" when the sentence passes the app's checks and names the key thing.
        "explain": { input, context in
            let parts = input.split(separator: "\n", maxSplits: 1).map(String.init)
            let e = try await OnDeviceJobs.explain(tool: parts[0], request: parts.count > 1 ? parts[1] : "", reason: "")
            let key = parts[0] == "Bash" ? "command" : (parts[0] == "WebFetch" ? "url" : "file_path")
            let risks = PermissionRisk.of(tool: parts[0], input: .object([key: .string(parts.count > 1 ? parts[1] : "")]))
            if let p = PermissionExplainer.problem(e.what, request: input, risks: risks) { return "\(p): \(e.what)" }
            let said = VoiceText.normalize(e.what)
            if !context.split(separator: "|").contains(where: { said.contains(VoiceText.normalize(String($0))) }) { return "lacks \"\(context)\": \(e.what)" }
            return "ok"
        },
        // Suggested replies in a chat: "none" when the message doesn't end on a question (no model call), else
        // "ok" when at least two replies are usable and one has the case's expected word.
        "chat_replies": { input, context in
            guard ChatReplies.asksSomething(input) else { return "none" }
            let a = try await OnDeviceJobs.assist(title: "The agent asked you something", text: input, language: VoiceText.dominantLanguage(input))
            let replies = ChatReplies.shown(OnDeviceJobs.usableReplies(a.replies, for: input), for: input)
            if replies.isEmpty { return "none shown: \(a.replies)" }
            if !context.isEmpty, !replies.contains(where: { VoiceText.normalize($0).contains(VoiceText.normalize(context)) }) {
                return "no reply with \"\(context)\": \(replies)"
            }
            return "ok"
        },
        // "ok" when the summary passes the app's checks (short, mostly the item's own words) and at least two replies
        // are usable and one has the case's expected option. The case's first key is kept for reading the misses.
        "assist": { input, context in
            let parts = input.split(separator: "\n", maxSplits: 1).map(String.init)
            let keys = context.split(separator: "|").map(String.init)
            let a = try await OnDeviceJobs.assist(title: parts[0], text: parts.count > 1 ? parts[1] : "", language: VoiceText.dominantLanguage(input))
            if let p = OnDeviceJobs.problem(withSummary: a.summary, for: input) { return "summary \(p): \(a.summary)" }
            let replies = OnDeviceJobs.usableReplies(a.replies, for: input)
            if replies.count < 2 { return "\(replies.count) usable replies: \(a.replies)" }
            if keys.count > 1, !replies.contains(where: { VoiceText.normalize($0).contains(VoiceText.normalize(keys[1])) }) {
                return "no reply with \"\(keys[1])\": \(replies)"
            }
            return "ok"
        },
        // "ok" when the acknowledgement passes the app's checks and mentions the case's keyword.
        "quick_ack": { input, keyword in
            let r = try await OnDeviceJobs.quickRead(input)
            if let p = OnDeviceJobs.problem(withSubject: r.subject, message: input) { return "fallback, \(p): \(r.subject)" }
            let ack = OnDeviceJobs.ack(subject: r.subject, language: VoiceText.dominantLanguage(input))
            if !keyword.isEmpty, !VoiceText.normalize(ack).contains(VoiceText.normalize(keyword)) { return "missing \"\(keyword)\": \(ack)" }
            return "ok"
        },
        // The agent it would offer "Send to …?" for, or Orchestrator for none.
        "route": { input, _ in
            guard !agents.isEmpty else { throw CocoaError(.fileReadNoSuchFile, userInfo: [NSFilePathErrorKey: "local/agents.json"]) }
            let r = try await OnDeviceJobs.route(input, agents: agents)
            return OnDeviceJobs.agent(for: r, agents: agents) ?? "Orchestrator"
        },
        // "ok", or why the spoken version is unusable (then the app reads its usual short version instead).
        // The case's context is a word the spoken version must keep.
        "spoken": { input, keep in
            let out = try await OnDeviceJobs.spoken(input, language: VoiceText.dominantLanguage(input))
            if let p = OnDeviceJobs.problem(withSpoken: out, for: input) { return "\(p): \(out)" }
            if !keep.isEmpty, !out.lowercased().contains(keep) { return "missing \"\(keep)\": \(out)" }
            return "ok"
        },
        // Today's behaviour, for comparison.
        "grill_matcher": { input, _ in GrillUnderstanding.label(GrillMatcher.match(input, phrases: phrases)) },
        "grill_pipeline": { input, question in
            let m = await GrillUnderstanding.read(input, question: question, phrases: phrases) { text, q in
                try? await OnDeviceJobs.grill(text, question: q).rawValue
            }
            return GrillUnderstanding.label(m)
        },
    ]
}
