import Foundation

/// Every failure the app shows, with a plain explanation and what to do next.
enum APIError: Error, Equatable, LocalizedError {
    case notConfigured
    case invalidURL(String)
    /// The host couldn't be reached at all: Tailscale off, wrong address, Mac asleep.
    case unreachable(String)
    case timeout
    case insecureConnection
    /// The server answered with an error. `message` comes from the backend's `{ "error": … }`.
    case server(status: Int, message: String)
    case decoding(String)
    /// This Loft server doesn't have the feature (an older backend).
    case unsupported(String)
    /// Writes are off while disconnected, so nothing runs later by surprise.
    case offline
    case cancelled

    var errorDescription: String? { title }

    var title: String {
        switch self {
        case .notConfigured: "No server set up"
        case .invalidURL: "That address isn't valid"
        case .unreachable: "Can't reach Loft"
        case .timeout: "Loft didn't answer in time"
        case .insecureConnection: "Secure connection failed"
        case .server(let status, let message):
            switch status {
            case 404: message == "not found" ? "Not found" : message
            case 409: message
            case 400, 422: message
            default: "Loft returned an error (\(status))"
            }
        case .decoding: "Unexpected response from Loft"
        case .unsupported(let what): "\(what) needs a newer Loft"
        case .offline: "You're offline"
        case .cancelled: "Cancelled"
        }
    }

    var suggestion: String {
        switch self {
        case .notConfigured: "Enter your Mac's Tailscale address in Settings."
        case .invalidURL: "Use the full address, like https://my-mac.tailnet-name.ts.net"
        case .unreachable: "Check that Tailscale is connected on this iPhone and that the Mac is awake and running Loft."
        case .timeout: "The Mac may be busy or asleep. Try again in a moment."
        case .insecureConnection: "Make sure the address starts with https:// and that `tailscale serve` is running on the Mac."
        case .server(let status, _): status >= 500 ? "Check the Loft logs on the Mac." : "Nothing was changed."
        case .decoding: "The app and the server may be on different versions. Update Loft on the Mac."
        case .unsupported: "Pull the latest Loft on the Mac and restart it."
        case .offline: "Reconnect to change anything. You can still browse what was last loaded."
        case .cancelled: ""
        }
    }

    /// For the diagnostics screen: specific, but never includes request bodies.
    var diagnostic: String {
        switch self {
        case .unreachable(let d), .decoding(let d), .invalidURL(let d): "\(title): \(d)"
        case .server(let status, let m): "HTTP \(status): \(m)"
        default: title
        }
    }

    static func from(_ error: Error) -> APIError {
        if let e = error as? APIError { return e }
        if error is CancellationError { return .cancelled }
        if let e = error as? URLError {
            switch e.code {
            case .cancelled: return .cancelled
            case .timedOut: return .timeout
            case .secureConnectionFailed, .serverCertificateUntrusted, .serverCertificateHasBadDate,
                 .serverCertificateNotYetValid, .serverCertificateHasUnknownRoot, .clientCertificateRejected,
                 .appTransportSecurityRequiresSecureConnection:
                return .insecureConnection
            default: return .unreachable(e.localizedDescription)
            }
        }
        if let e = error as? DecodingError { return .decoding(String(describing: e).prefix(300).description) }
        return .unreachable(error.localizedDescription)
    }
}
