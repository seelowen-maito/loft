import Foundation

/// "#18" being typed at the end of the message: the specs, issues and pull requests to offer, and the text once one
/// is picked. The same rules as the web composer (frontend lib/shortcuts.ts) and the tag text the backend reads
/// (`#N` in the agent's project, `owner/repo#N` elsewhere, the id for a Loft-only issue; see backend tags.rs).
public enum TagQuery {
    public enum Kind: String { case spec, issue, pr }

    public struct Row: Identifiable, Hashable {
        public let kind: Kind
        public let title: String
        public let tag: String
        public let open: Bool
        public let project: String
        public let number: Int?
        public var id: String { "\(kind.rawValue):\(tag)" }
    }

    /// What follows a `#` that starts a word at the end of `text`, or nil when no tag is being typed.
    public static func query(in text: String) -> String? {
        guard let hash = text.lastIndex(of: "#") else { return nil }
        if let before = text[..<hash].last, !(before.isWhitespace || "([{\"'".contains(before)) { return nil }
        let q = text[text.index(after: hash)...]
        guard !q.contains(where: { $0.isWhitespace || $0 == "#" }) else { return nil }
        return String(q)
    }

    /// Every spec, issue and pull request, each with the tag to insert for it. `project` is the chat agent's.
    public static func rows(tasks: [HBTask], prs: [PullRequest], project: String) -> [Row] {
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
    /// open ones of each; otherwise the web's ⌘K search (frontend lib/shortcuts.ts `searchRows`) picks them.
    public static func matches(_ rows: [Row], _ q: String) -> [Row] {
        let blank = q.trimmingCharacters(in: .whitespaces).isEmpty
        let per = blank ? 3 : 4
        return [Kind.spec, .issue, .pr].flatMap { kind in
            let of = rows.filter { $0.kind == kind }
            return blank ? Array(of.filter(\.open).prefix(per)) : search(of, q, limit: per)
        }
    }

    /// `searchRows` from the web: `#123` or `123` finds that number. Otherwise every word must appear in the title or
    /// project; open rows come first, then titles that start with the query, then titles containing it.
    public static func search(_ rows: [Row], _ query: String, limit: Int) -> [Row] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if q.isEmpty { return [] }
        let digits = q.hasPrefix("#") ? q.dropFirst() : Substring(q)
        if !digits.isEmpty, digits.allSatisfy(\.isASCII), digits.allSatisfy(\.isNumber), let n = Int(digits) {
            return Array(rows.filter { $0.number == n }.enumerated().sorted { ($0.element.open ? 0 : 1, $0.offset) < ($1.element.open ? 0 : 1, $1.offset) }.map(\.element).prefix(limit))
        }
        let words = q.split(whereSeparator: \.isWhitespace)
        func score(_ r: Row) -> Int {
            let title = r.title.lowercased()
            return (r.open ? 0 : 3) + (title.hasPrefix(q) ? 0 : title.contains(q) ? 1 : 2)
        }
        let found = rows.filter { r in let text = "\(r.title) \(r.project)".lowercased(); return words.allSatisfy { text.contains($0) } }
        return Array(found.enumerated().sorted { (score($0.element), $0.offset) < (score($1.element), $1.offset) }.map(\.element).prefix(limit))
    }

    /// `text` with the `#…` being typed replaced by the tag and a space.
    public static func insert(_ tag: String, into text: String) -> String {
        guard let hash = text.lastIndex(of: "#") else { return text + "\(tag) " }
        return String(text[..<hash]) + "\(tag) "
    }
}
