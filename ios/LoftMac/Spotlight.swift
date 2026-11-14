import AppKit
@preconcurrency import CoreSpotlight
import LoftKit
import Observation

/// Loft in macOS Spotlight (spec #456): keeps the agents and open issues, tickets and specs in Core Spotlight as the
/// store changes, for as long as the app runs. Settings › Spotlight turns it off, which clears them.
@MainActor
final class MacSpotlight {
    private let store: LoftStore
    private let index = CSSearchableIndex.default()
    /// What the index holds, as far as this run knows; nil until the first sync, which starts from a cleared index.
    private var indexed: [QuickSearch.Result]?

    init(store: LoftStore) {
        self.store = store
        NotificationCenter.default.addObserver(forName: UserDefaults.didChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.sync() }
        }
        watch()
    }

    private func watch() {
        withObservationTracking { sync() } onChange: { [weak self] in
            Task { @MainActor in self?.watch() }
        }
    }

    private func sync() {
        let results = SpotlightIndex.results(agents: store.agents, tasks: store.tasks, deliveries: Array(store.deliverySummaries.values))
        guard UserDefaults.standard.bool(forKey: Prefs.spotlight) else {
            if indexed != [] { index.deleteSearchableItems(withDomainIdentifiers: [SpotlightIndex.domain]) }
            indexed = []
            return
        }
        // Nothing loaded yet: keep what's there rather than clear it.
        guard !results.isEmpty else { return }
        guard let indexed else {
            // Items closed or deleted while the app wasn't running go too.
            index.deleteSearchableItems(withDomainIdentifiers: [SpotlightIndex.domain]) { [index] _ in
                index.indexSearchableItems(results.map(SpotlightIndex.item))
            }
            self.indexed = results
            return
        }
        let change = SpotlightIndex.changes(from: indexed, to: results)
        if !change.remove.isEmpty { index.deleteSearchableItems(withIdentifiers: change.remove) }
        if !change.index.isEmpty { index.indexSearchableItems(change.index.map(SpotlightIndex.item)) }
        self.indexed = results
    }
}

/// Opens a Loft item chosen in Spotlight.
final class MacAppDelegate: NSObject, NSApplicationDelegate {
    /// Set by the app once quick search, which knows how to open an item, exists.
    @MainActor static var openSpotlightItem: ((String) -> Void)?

    func application(_ application: NSApplication, continue activity: NSUserActivity,
                     restorationHandler: @escaping ([any NSUserActivityRestoring]) -> Void) -> Bool {
        guard activity.activityType == CSSearchableItemActionType,
              let id = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String else { return false }
        MainActor.assumeIsolated { Self.openSpotlightItem?(id) }
        return true
    }
}
