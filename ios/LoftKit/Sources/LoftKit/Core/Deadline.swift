import Foundation

/// `work`'s result, or nil when it hasn't finished after `seconds`. Unlike a task group, a `work` that ignores
/// cancellation (or never finishes) can't hold the caller up: it's left running and its result dropped.
public func within<T: Sendable>(seconds: Double, _ work: @escaping @Sendable () async -> T) async -> T? {
    await withCheckedContinuation { (continuation: CheckedContinuation<T?, Never>) in
        let once = Once(continuation)
        Task { once.resume(await work()) }
        Task { try? await Task.sleep(for: .seconds(seconds)); once.resume(nil) }
    }
}

/// A continuation resumed by whoever gets there first.
private final class Once<T: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<T?, Never>?

    init(_ continuation: CheckedContinuation<T?, Never>) { self.continuation = continuation }

    func resume(_ value: T?) {
        lock.lock()
        let c = continuation
        continuation = nil
        lock.unlock()
        c?.resume(returning: value)
    }
}
