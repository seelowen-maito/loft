import Foundation

/// The checks between the model's explanation of a permission request and the approval card.
enum PermissionExplainer {
    /// Words that would make a request sound safe: never shown, whatever the request is.
    private static let reassuring = [#"\bsafe(ly)?\b"#, #"\bharmless\b"#, #"\bno risk\b"#, #"\bnothing (bad|dangerous|harmful)\b"#,
                                     #"\bwon'?t (hurt|harm|break|affect)\b"#, #"\bonly reads?\b"#, #"\bjust\b"#, #"\bsimply\b"#,
                                     #"\bseguro\b"#, #"\binofensiv"#, #"\bsem risco\b"#, #"\bapenas\b"#, #"\bs[óo] l[êe]\b"#]

    /// The sentence as shown: without a leading "Bash:" or "This would", capitalised.
    static func cleaned(_ s: String, tool: String) -> String {
        var t = s.trimmingCharacters(in: .whitespacesAndNewlines)
        for prefix in ["\(tool):", "\(tool) allows", "\(tool) would", "This would", "Allowing this would"] where t.lowercased().hasPrefix(prefix.lowercased()) {
            t = String(t.dropFirst(prefix.count)).trimmingCharacters(in: .whitespaces)
        }
        return t.prefix(1).uppercased() + t.dropFirst()
    }

    /// Words that say a request changes or sends something, per risk the code found. A sentence for a request that
    /// deletes, overwrites or sends has to say so: "reads the file" for a Write would undersell it.
    private static let mustSay: [PermissionRisk: [String]] = [
        .deletes: ["delet", "remov", "overwrit", "replac", "erase", "wipe", "chang", "writ", "edit", "reset", "clean", "apag", "substitu", "alter"],
        .sendsMessages: ["send", "sent", "post", "comment", "message", "email", "mail", "text", "envi", "mensag"],
        .pushesToMain: ["main", "merg", "push", "send", "upload"],
    ]

    /// Why an explanation can't be shown, or nil. Paths and numbers in it have to be in the request, and it has to
    /// agree with the risks the code found.
    static func problem(_ s: String, request: String, risks: [PermissionRisk] = []) -> String? {
        let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
        let lower = t.lowercased()
        for r in risks { if let words = mustSay[r], !words.contains(where: lower.contains) { return "doesn't say it \(r.label.lowercased())" } }
        if t.count < 12 { return "too short" }
        if t.split(separator: " ").count > 30 { return "too long" }
        if reassuring.contains(where: { t.range(of: $0, options: [.regularExpression, .caseInsensitive]) != nil }) { return "sounds reassuring" }
        if t.contains("`") { return "has code" }
        let req = request.lowercased()
        for w in t.lowercased().split(whereSeparator: { $0 == " " || $0 == "," || $0 == "(" || $0 == ")" }) {
            let token = w.trimmingCharacters(in: CharacterSet(charactersIn: ".:;\"'"))
            if (token.contains("/") || token.contains(where: \.isNumber)), !req.contains(token) { return "names something not in the request (\(token))" }
        }
        return nil
    }
}
