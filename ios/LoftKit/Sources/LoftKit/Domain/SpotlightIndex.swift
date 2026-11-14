import CoreSpotlight
import Foundation
import UniformTypeIdentifiers

/// Loft in macOS Spotlight (spec #456): the agents and open issues, tickets and specs, as quick search's results, made
/// into Core Spotlight items whose identifier (`agent:<id>`, `ticket:<id>`…) says what to open when one is chosen.
public enum SpotlightIndex {
    /// Every item Loft puts in Spotlight shares this domain, so they can be cleared together.
    public static let domain = "loft"

    /// What goes in the index: every agent, and the issues, tickets and specs that aren't closed.
    public static func results(agents: [Agent], tasks: [HBTask], deliveries: [DeliverySummary]) -> [QuickSearch.Result] {
        agents.map { QuickSearch.Result(kind: .agent, target: $0.id, title: $0.name, webPath: "/agents/\($0.id)") }
            + tasks.filter { !$0.state.isClosed }.map { QuickSearch.result(for: $0, deliveries: deliveries) }
    }

    public static func item(_ r: QuickSearch.Result) -> CSSearchableItem {
        let a = CSSearchableItemAttributeSet(contentType: .content)
        a.title = r.title
        a.displayName = r.title
        a.contentDescription = description(r)
        // "460" and "#460" find a ticket too.
        let n = r.number.flatMap { $0.split(separator: "#").last }.map(String.init)
        a.keywords = [r.kind.label] + (n.map { [$0, "#" + $0] } ?? [])
        return CSSearchableItem(uniqueIdentifier: r.id, domainIdentifier: domain, attributeSet: a)
    }

    /// "Agent", or "Ticket · loft#460".
    static func description(_ r: QuickSearch.Result) -> String {
        [r.kind.label, r.number].compactMap { $0 }.joined(separator: " · ")
    }

    /// The kind and id a chosen item's identifier names; nil for one Loft doesn't know.
    public static func target(of identifier: String) -> (kind: QuickSearch.Kind, id: String)? {
        guard let colon = identifier.firstIndex(of: ":"), let kind = QuickSearch.Kind(rawValue: String(identifier[..<colon])),
              kind != .page else { return nil }
        let id = String(identifier[identifier.index(after: colon)...])
        return id.isEmpty ? nil : (kind, id)
    }

    /// What to (re)index and which identifiers to remove, going from what's in the index to what should be.
    public static func changes(from old: [QuickSearch.Result], to new: [QuickSearch.Result])
        -> (index: [QuickSearch.Result], remove: [String]) {
        let before = Dictionary(old.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let now = Set(new.map(\.id))
        return (new.filter { before[$0.id] != $0 }, before.keys.filter { !now.contains($0) }.sorted())
    }
}
