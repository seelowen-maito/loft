import CoreSpotlight
import Foundation
import UniformTypeIdentifiers

/// Loft in macOS Spotlight (spec #456): the agents and open issues, tickets and specs, as quick search's results, made
/// into Core Spotlight items whose identifier (`agent:<id>`, `ticket:<id>`…) says what to open when one is chosen.
public enum SpotlightIndex {
    /// Every item Loft puts in Spotlight shares this domain, so they can be cleared together.
    public static let domain = "loft"

    /// One item to index: quick search's result, plus an agent's line and sticker.
    public struct Entry: Hashable, Sendable {
        public var result: QuickSearch.Result
        /// "loft · Dev" for an agent; "Ticket · loft#460" otherwise.
        public var detail: String
        /// The agent's sticker SVG, nil without one or for an issue.
        public var svg: String?
        public var lead = false
        public var id: String { result.id }
    }

    /// What goes in the index: every agent, and the issues, tickets and specs that aren't closed.
    public static func entries(agents: [Agent], tasks: [HBTask], deliveries: [DeliverySummary]) -> [Entry] {
        agents.map { a in
            let r = QuickSearch.result(for: a)
            return Entry(result: r, detail: r.detail ?? "", svg: a.avatar?.svg, lead: a.isLead)
        } + tasks.filter { !$0.state.isClosed }.map { t in
            let r = QuickSearch.result(for: t, deliveries: deliveries)
            return Entry(result: r, detail: [r.kind.label, r.number].compactMap { $0 }.joined(separator: " · "))
        }
    }

    /// The sticker's size in Spotlight's results, in pixels (2x for a 64pt image).
    static let thumbnailPixels = 128

    public static func item(_ e: Entry) -> CSSearchableItem {
        let r = e.result
        let a = CSSearchableItemAttributeSet(contentType: .content)
        a.title = r.title
        a.displayName = r.title
        a.contentDescription = e.detail
        // "460" and "#460" find a ticket too.
        let n = r.number.flatMap { $0.split(separator: "#").last }.map(String.init)
        a.keywords = [r.kind.label] + (n.map { [$0, "#" + $0] } ?? [])
        a.thumbnailData = e.svg.flatMap { Sticker.png(svg: $0, pixels: thumbnailPixels, lead: e.lead) }
        return CSSearchableItem(uniqueIdentifier: r.id, domainIdentifier: domain, attributeSet: a)
    }

    /// The kind and id a chosen item's identifier names; nil for one Loft doesn't know.
    public static func target(of identifier: String) -> (kind: QuickSearch.Kind, id: String)? {
        guard let colon = identifier.firstIndex(of: ":"), let kind = QuickSearch.Kind(rawValue: String(identifier[..<colon])),
              kind != .page, kind != .pullRequest else { return nil }
        let id = String(identifier[identifier.index(after: colon)...])
        return id.isEmpty ? nil : (kind, id)
    }

    /// What to (re)index and which identifiers to remove, going from what's in the index to what should be.
    public static func changes(from old: [Entry], to new: [Entry]) -> (index: [Entry], remove: [String]) {
        let before = Dictionary(old.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let now = Set(new.map(\.id))
        return (new.filter { before[$0.id] != $0 }, before.keys.filter { !now.contains($0) }.sorted())
    }
}
