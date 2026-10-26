import Foundation

/// Incremental server-sent-events parser: feed it lines, get back each event's
/// `data` once the blank line that ends the event arrives.
struct SSEParser {
    private var data: [String] = []

    mutating func feed(_ line: String) -> String? {
        if line.isEmpty {
            defer { data.removeAll() }
            return data.isEmpty ? nil : data.joined(separator: "\n")
        }
        if line.hasPrefix(":") { return nil } // comment / keep-alive
        let (field, value): (Substring, Substring)
        if let colon = line.firstIndex(of: ":") {
            field = line[..<colon]
            var v = line[line.index(after: colon)...]
            if v.hasPrefix(" ") { v = v.dropFirst() }
            value = v
        } else {
            field = Substring(line)
            value = ""
        }
        if field == "data" { data.append(String(value)) }
        return nil
    }
}

/// Exponential backoff with full jitter, capped.
struct Backoff {
    var base: TimeInterval = 1
    var cap: TimeInterval = 10
    private(set) var attempt = 0

    init(base: TimeInterval = 1, cap: TimeInterval = 10) {
        self.base = base
        self.cap = cap
    }

    /// The delay before the next attempt; grows each call.
    mutating func next(random: (ClosedRange<Double>) -> Double = { Double.random(in: $0) }) -> TimeInterval {
        let ceiling = min(cap, base * pow(2, Double(attempt)))
        attempt += 1
        return random(ceiling / 2 ... ceiling)
    }

    mutating func reset() { attempt = 0 }
}
