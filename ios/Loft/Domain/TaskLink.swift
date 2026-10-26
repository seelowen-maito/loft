import Foundation

/// Task links: the app's own `loft://task/<id>`, and the web app's `…/tasks/<id>` (the forms the backend
/// recognises in a message, #116).
enum TaskLink {
    static func url(_ id: String) -> URL { URL(string: "loft://task/\(id)")! }

    /// The task a link points to, or nil.
    static func id(from url: URL) -> String? {
        if url.scheme == "loft", url.host == "task" {
            let id = url.pathComponents.dropFirst().first ?? ""
            return id.isEmpty ? nil : id.lowercased()
        }
        let parts = url.pathComponents
        if let i = parts.lastIndex(of: "tasks"), i + 1 < parts.count, !parts[i + 1].isEmpty { return parts[i + 1].lowercased() }
        return nil
    }
}
