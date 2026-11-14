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
    private var indexed: [SpotlightIndex.Entry]?
    /// The index work so far, run in order and off the main actor: drawing stickers can take a while.
    private var work: Task<Void, Never>?

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
        let results = SpotlightIndex.entries(agents: store.agents, tasks: store.tasks, deliveries: Array(store.deliverySummaries.values))
        guard UserDefaults.standard.bool(forKey: Prefs.spotlight) else {
            if indexed != [] { enqueue(clear: true) }
            indexed = []
            return
        }
        // Nothing loaded yet: keep what's there rather than clear it.
        guard !results.isEmpty else { return }
        guard let indexed else {
            // Items closed or deleted while the app wasn't running go too.
            enqueue(clear: true, index: results)
            self.indexed = results
            return
        }
        let change = SpotlightIndex.changes(from: indexed, to: results)
        if !change.remove.isEmpty || !change.index.isEmpty { enqueue(remove: change.remove, index: change.index) }
        self.indexed = results
    }

    private func enqueue(clear: Bool = false, remove: [String] = [], index entries: [SpotlightIndex.Entry] = []) {
        let previous = work, index = index
        work = Task.detached(priority: .utility) {
            await previous?.value
            if clear { try? await index.deleteSearchableItems(withDomainIdentifiers: [SpotlightIndex.domain]) }
            if !remove.isEmpty { try? await index.deleteSearchableItems(withIdentifiers: remove) }
            if !entries.isEmpty { try? await index.indexSearchableItems(entries.map(SpotlightIndex.item)) }
        }
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
