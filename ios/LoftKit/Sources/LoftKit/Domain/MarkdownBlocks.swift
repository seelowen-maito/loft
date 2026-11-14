import Foundation

/// A chat message's Markdown split into blocks, as the web's react-markdown with GFM draws them: headings, lists,
/// fenced code, quotes, tables and rules. Each block's text is inline Markdown (bold, links, `code`), which the
/// view draws with `AttributedString(markdown:)`.
public indirect enum MarkdownBlock: Equatable, Sendable {
    case heading(level: Int, text: String)
    case paragraph(String)
    case list(ordered: Bool, items: [ListItem])
    case code(language: String, text: String)
    case quote([MarkdownBlock])
    case table(header: [String], alignments: [ColumnAlignment], rows: [[String]])
    case rule

    /// A table column's alignment from its divider: `:---:` centres, `---:` aligns right.
    public enum ColumnAlignment: Equatable, Sendable { case leading, center, trailing }

    /// How deep quotes nest before further `>` stay as text, so a line of thousands of `>` can't overflow the stack.
    static let maxDepth = 8

    public struct ListItem: Equatable, Sendable {
        /// How deep it's nested: 0 for a top-level item.
        public var depth: Int
        /// "1." for a numbered item, "•" otherwise.
        public var marker: String
        /// A task list item's box: nil when it has none.
        public var checked: Bool?
        public var text: String
        /// Blocks indented under the item after its first paragraph, such as a fenced command in a numbered step.
        public var blocks: [MarkdownBlock]

        public init(depth: Int, marker: String, checked: Bool? = nil, text: String, blocks: [MarkdownBlock] = []) {
            self.depth = depth; self.marker = marker; self.checked = checked; self.text = text; self.blocks = blocks
        }
    }

    public static func parse(_ markdown: String) -> [MarkdownBlock] {
        var lines = markdown.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")[...]
        return parse(&lines, depth: 0)
    }

    private static func parse(_ lines: inout ArraySlice<String>, depth: Int) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        var paragraph = ""
        func flush() {
            if paragraph.hasSuffix("\n") { paragraph.removeLast() }
            if !paragraph.isEmpty { blocks.append(.paragraph(paragraph)) }
            paragraph = ""
        }
        while let line = lines.first {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.isEmpty {
                flush(); lines.removeFirst()
            } else if let fence = fence(trimmed) {
                flush(); lines.removeFirst()
                let language = String(trimmed.dropFirst(fence.count)).trimmingCharacters(in: .whitespaces)
                let indent = line.prefix { $0 == " " }.count
                var code: [String] = []
                while let l = lines.popFirst(), !l.trimmingCharacters(in: .whitespaces).hasPrefix(fence) {
                    code.append(String(l.dropFirst(min(indent, l.prefix { $0 == " " }.count))))
                }
                blocks.append(.code(language: language, text: code.joined(separator: "\n")))
            } else if let (level, text) = heading(trimmed) {
                flush(); lines.removeFirst()
                blocks.append(.heading(level: level, text: text))
            } else if isRule(trimmed) {
                flush(); lines.removeFirst()
                blocks.append(.rule)
            } else if trimmed.hasPrefix(">"), depth < maxDepth {
                flush()
                var quoted: [String] = []
                while let l = lines.first?.trimmingCharacters(in: .whitespaces), l.hasPrefix(">") {
                    var rest = l.dropFirst()
                    if rest.first == " " { rest = rest.dropFirst() }
                    quoted.append(String(rest)); lines.removeFirst()
                }
                var inner = quoted[...]
                blocks.append(.quote(parse(&inner, depth: depth + 1)))
            } else if listItem(line) != nil {
                flush()
                blocks.append(list(&lines, depth: depth))
            } else if trimmed.contains("|"), lines.dropFirst().first.map(isTableDivider) == true {
                flush()
                let header = cells(trimmed)
                let alignments = cells(lines.dropFirst().first!.trimmingCharacters(in: .whitespaces)).map { c -> ColumnAlignment in
                    c.hasSuffix(":") ? (c.hasPrefix(":") ? .center : .trailing) : .leading
                }
                lines.removeFirst(2)
                var rows: [[String]] = []
                while let l = lines.first?.trimmingCharacters(in: .whitespaces), l.contains("|") {
                    rows.append(cells(l)); lines.removeFirst()
                }
                blocks.append(.table(header: header, alignments: alignments, rows: rows))
            } else {
                // As GFM: lines of a paragraph join with a space; two trailing spaces or a `\\` break the line.
                if !paragraph.isEmpty { paragraph += paragraph.hasSuffix("\n") ? "" : " " }
                if line.hasSuffix("  ") { paragraph += trimmed + "\n" }
                else if trimmed.hasSuffix("\\") { paragraph += trimmed.dropLast() + "\n" }
                else { paragraph += trimmed }
                lines.removeFirst()
            }
        }
        flush()
        return blocks
    }

    /// A run of list items, numbered or not as the first one is. Lines indented under an item are parsed as its
    /// own blocks: its first paragraph is the item's text, and a fenced command or a quote below it stays inside it.
    private static func list(_ lines: inout ArraySlice<String>, depth: Int) -> MarkdownBlock {
        let ordered = listItem(lines.first!)!.ordered
        var items: [ListItem] = []
        var bodies: [[String]] = []
        var indents: [Int] = []
        var contentIndent = 0
        func isIndented(_ line: String) -> Bool { line.hasPrefix(" ") || line.hasPrefix("\t") }
        func dedent(_ line: String) -> String {
            String(line.dropFirst(min(contentIndent, line.prefix { $0 == " " }.count)))
        }
        while let line = lines.first {
            let blank = line.trimmingCharacters(in: .whitespaces).isEmpty
            if let item = listItem(line) {
                // A top-level item of the other kind starts a new list.
                if item.ordered != ordered, item.indent <= indents.first ?? 0 { break }
                // Each deeper indent is one more level; a shallower one goes back to the level it matches.
                while let last = indents.last, item.indent < last { indents.removeLast() }
                if indents.last.map({ item.indent > $0 }) ?? true { indents.append(item.indent) }
                items.append(ListItem(depth: indents.count - 1, marker: item.marker, checked: item.checked, text: ""))
                bodies.append([item.text])
                contentIndent = item.contentIndent
                lines.removeFirst()
            } else if !items.isEmpty, isIndented(line), !blank {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                bodies[bodies.count - 1].append(dedent(line)); lines.removeFirst()
                // A fence keeps its lines, blank or looking like items, up to its closing fence.
                if let fence = fence(trimmed) {
                    while let l = lines.popFirst() {
                        bodies[bodies.count - 1].append(dedent(l))
                        if l.trimmingCharacters(in: .whitespaces).hasPrefix(fence) { break }
                    }
                }
            } else if blank, let next = lines.dropFirst().first, listItem(next) != nil || (!items.isEmpty && isIndented(next)) {
                // A blank line between items, or before more of an item, keeps the list going.
                if !items.isEmpty { bodies[bodies.count - 1].append("") }
                lines.removeFirst()
            } else {
                break
            }
        }
        for i in items.indices {
            var body = bodies[i][...]
            var blocks = parse(&body, depth: depth + 1)
            if case .paragraph(let text) = blocks.first { items[i].text = text; blocks.removeFirst() }
            items[i].blocks = blocks
        }
        return .list(ordered: ordered, items: items)
    }

    private static func listItem(_ line: String) -> (indent: Int, contentIndent: Int, ordered: Bool, marker: String, checked: Bool?, text: String)? {
        let indent = line.prefix { $0 == " " || $0 == "\t" }.reduce(0) { $0 + ($1 == "\t" ? 4 : 1) }
        let rest = line.drop { $0 == " " || $0 == "\t" }
        var ordered = false, marker = "•", body: Substring
        if let c = rest.first, "-*+".contains(c), rest.dropFirst().first == " " {
            body = rest.dropFirst(2)
        } else {
            let digits = rest.prefix { $0.isNumber }
            guard !digits.isEmpty, digits.count < 10, let d = rest.dropFirst(digits.count).first, d == "." || d == ")",
                  rest.dropFirst(digits.count + 1).first == " " else { return nil }
            ordered = true; marker = "\(Int(digits) ?? 1)."
            body = rest.dropFirst(digits.count + 2)
        }
        let contentIndent = indent + (rest.count - body.count)
        var checked: Bool?
        if body.hasPrefix("[ ] ") { checked = false; body = body.dropFirst(4) }
        if body.lowercased().hasPrefix("[x] ") { checked = true; body = body.dropFirst(4) }
        return (indent, contentIndent, ordered, marker, checked, body.trimmingCharacters(in: .whitespaces))
    }

    private static func fence(_ trimmed: String) -> String? {
        ["```", "~~~"].first { trimmed.hasPrefix($0) }
    }

    private static func heading(_ trimmed: String) -> (Int, String)? {
        let hashes = trimmed.prefix { $0 == "#" }.count
        guard (1...6).contains(hashes), trimmed.count == hashes || trimmed.dropFirst(hashes).first == " " else { return nil }
        var text = trimmed.dropFirst(hashes).trimmingCharacters(in: .whitespaces)
        while text.hasSuffix("#") { text.removeLast() }
        return (hashes, text.trimmingCharacters(in: .whitespaces))
    }

    private static func isRule(_ trimmed: String) -> Bool {
        let s = trimmed.filter { $0 != " " }
        return s.count >= 3 && ["-", "*", "_"].contains { c in s.allSatisfy { String($0) == c } }
    }

    private static func isTableDivider(_ line: String) -> Bool {
        let s = line.trimmingCharacters(in: .whitespaces)
        return s.contains("-") && s.allSatisfy { "|-: ".contains($0) }
    }

    private static func cells(_ row: String) -> [String] {
        var s = Substring(row)
        if s.hasPrefix("|") { s = s.dropFirst() }
        if s.hasSuffix("|") { s = s.dropLast() }
        return s.split(separator: "|", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
    }
}
