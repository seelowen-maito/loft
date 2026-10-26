import Foundation

/// What a permission request could do that you'd want to know before allowing it. Decided in code from the tool
/// and its input, never by the model, so a risky request can't be made to look safe.
enum PermissionRisk: String, CaseIterable, Identifiable {
    case deletes, secrets, sendsMessages, spendsMoney, network, pushesToMain

    var id: String { rawValue }

    var label: String {
        switch self {
        case .deletes: "Deletes or overwrites"
        case .secrets: "Touches secrets"
        case .sendsMessages: "Sends messages"
        case .spendsMoney: "Can cost money"
        case .network: "Uses the network"
        case .pushesToMain: "Pushes to main"
        }
    }

    var symbol: String {
        switch self {
        case .deletes: "trash"
        case .secrets: "key"
        case .sendsMessages: "paperplane"
        case .spendsMoney: "creditcard"
        case .network: "network"
        case .pushesToMain: "arrow.up.to.line"
        }
    }

    /// Red for what can't be undone or leaks; amber for the rest.
    var serious: Bool { self == .deletes || self == .secrets || self == .pushesToMain || self == .spendsMoney }

    private static func has(_ s: String, _ patterns: [String]) -> Bool {
        patterns.contains { s.range(of: $0, options: [.regularExpression, .caseInsensitive]) != nil }
    }

    /// The risks in a request: its tool name and every string in its input.
    static func of(tool: String, input: JSONValue?, rule: String? = nil) -> [PermissionRisk] {
        let text = ([tool, rule ?? ""] + strings(input)).joined(separator: "\n")
        var out: [PermissionRisk] = []
        if has(text, [#"\brm\s"#, #"\brmdir\b"#, #"\bgit\s+(reset\s+--hard|clean\s+-\w*f|branch\s+-D|push\s+[^\n]*(--force|-f\b|--delete|:))"#,
                      #"\bdrop\s+(table|database)\b"#, #"\btruncate\b"#, #"\bdelete\b"#, #"\bunlink\b"#, #"\bshred\b"#, #"\bmkfs\b"#,
                      #"\bdd\s+if="#, #"(^|\n)(Write|MultiEdit)\b"#, #"\bgh\s+(repo|release|pr)\s+delete\b"#, #"\bkubectl\s+delete\b"#])
            || ["Write", "MultiEdit"].contains(tool) { out.append(.deletes) }
        if has(text, [#"\.env\b"#, #"\bsecrets?\b"#, #"\btoken\b"#, #"\bpassword\b"#, #"\bpasswd\b"#, #"\bapi[_-]?key\b"#, #"\.p8\b"#, #"\.pem\b"#,
                      #"\bid_(rsa|ed25519)\b"#, #"\.ssh/"#, #"\bkeychain\b"#, #"\bsecurity\s+find-"#, #"\bcredentials?\b"#, #"\bprivate[_ -]?key\b"#,
                      #"\bprintenv\b"#, #"(^|\s)env(\s|$)"#, #"\baws\s+configure\b"#]) { out.append(.secrets) }
        if has(text, [#"\bimessage\b"#, #"\bosascript\b[^\n]*\bMessages\b"#, #"\bsendmail\b"#, #"\bmail\s+-s\b"#, #"\bslack\b"#, #"\bsms\b"#,
                      #"\btwilio\b"#, #"\bgh\s+(pr|issue)\s+(comment|create|review)\b"#, #"\bsend_?(message|email)\b"#, #"\bwebhook\b"#]) { out.append(.sendsMessages) }
        if has(text, [#"\bstripe\b"#, #"\bpurchase\b"#, #"\bbilling\b"#, #"\bcharge\b"#, #"\bpayment\b"#, #"\baws\s+(ec2\s+run|lambda\s+create)"#,
                      #"\bgcloud\s+[^\n]*create\b"#, #"\baz\s+[^\n]*create\b"#, #"\bterraform\s+apply\b"#, #"\bnpm\s+publish\b"#]) { out.append(.spendsMoney) }
        if has(text, [#"\bcurl\b"#, #"\bwget\b"#, #"\bssh\s"#, #"\bscp\s"#, #"\brsync\s[^\n]*:"#, #"\bgit\s+(push|pull|fetch|clone)\b"#,
                      #"\bnpm\s+(install|i|publish)\b"#, #"\bpip3?\s+install\b"#, #"\bbrew\s+install\b"#, #"\bgh\s"#, #"(^|\n)WebFetch\b"#,
                      #"(^|\n)WebSearch\b"#, #"https?://"#, #"\bdeploy\b"#, #"\bnc\s"#]) || ["WebFetch", "WebSearch"].contains(tool) { out.append(.network) }
        if has(text, [#"\bgit\s+push\b[^\n]*\b(main|master)\b"#, #"\bgh\s+pr\s+merge\b"#]) { out.append(.pushesToMain) }
        return out
    }

    private static func strings(_ v: JSONValue?) -> [String] {
        switch v {
        case .string(let s)?: [s]
        case .array(let a)?: a.flatMap { strings($0) }
        case .object(let o)?: o.values.flatMap { strings($0) }
        default: []
        }
    }
}
