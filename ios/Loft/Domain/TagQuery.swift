import Foundation

/// "#18" being typed at the end of the message: the specs, issues and pull requests to offer, and the text once one
/// is picked. The same rules as the web composer (frontend lib/shortcuts.ts) and the tag text the backend reads
/// (`#N` in the agent's project, `owner/repo#N` elsewhere, the id for a Loft-only issue; see backend tags.rs).
enum TagQuery {
    enum Kind: String { case spec, issue, pr }

    struct Row: Identifiable, Hashable {
        let kind: Kind
        let title: String
        let tag: String
        let open: Bool
        let project: String
        let number: Int?
        var id: String { "\(kind.rawValue):\(tag)" }
    }

    /// What follows a `#` that starts a word at the end of `text`, or nil when no tag is being typed.
    static func query(in text: String) -> String? {
        guard let hash = text.lastIndex(of: "#") else { return nil }
        if let before = text[..<hash].last, !(before.isWhitespace || "([{\"'".contains(before)) { return nil }
        let q = text[text.index(after: hash)...]
        guard !q.contains(where: { $0.isWhitespace || $0 == "#" }) else { return nil }
        return String(q)
    }

    /// Every spec, issue and pull request, each with the tag to insert for it. `project` is the chat agent's.
    static func rows(tasks: [HBTask], prs: [PullRequest], project: String) -> [Row] {
        let mine = project.trimmingCharacters(in: .whitespaces)
        func tag(_ repo: String, _ number: Int, _ name: String) -> String { !mine.isEmpty && name == mine ? "#\(number)" : "\(repo)#\(number)" }
        let issues = tasks.map { t in
            Row(kind: (t.labels ?? []).contains { $0.lowercased() == "spec" } ? .spec : .issue, title: t.title,
                tag: t.github.map { tag($0.repo, $0.number, t.project) } ?? "#\(t.id)",
                open: !t.state.isClosed, project: t.project, number: t.github?.number)
        }
        // The same PR can be registered by more than one agent.
        var seen = Set<String>()
        let pulls = prs.filter { seen.insert($0.url).inserted }.map { p in
            let name = p.slug.split(separator: "/").last.map(String.init) ?? p.slug
            return Row(kind: .pr, title: p.title, tag: tag(p.slug, p.number, name), open: p.state == "open" || p.state.isEmpty, project: name, number: p.number)
        }
        return issues + pulls
    }

    /// The picker's rows for what's typed after `#`: specs, issues, then pull requests. Nothing typed lists a few
    /// open ones of each; otherwise titles containing the text, or the number.
    static func matches(_ rows: [Row], _ q: String) -> [Row] {
        let q = q.trimmingCharacters(in: .whitespaces).lowercased()
        let per = q.isEmpty ? 3 : 4
        return [Kind.spec, .issue, .pr].flatMap { kind in
            rows.filter { $0.kind == kind && (q.isEmpty ? $0.open : $0.title.lowercased().contains(q) || $0.number.map { String($0).hasPrefix(q) } == true) }.prefix(per)
        }
    }

    /// `text` with the `#…` being typed replaced by the tag and a space.
    static func insert(_ tag: String, into text: String) -> String {
        guard let hash = text.lastIndex(of: "#") else { return text + "\(tag) " }
        return String(text[..<hash]) + "\(tag) "
    }
}
