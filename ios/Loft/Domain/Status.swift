import Foundation

/// What an agent is doing, normalized from the backend's status string, whether a
/// process is running, and its pending approvals. Only states Loft can actually
/// report are modelled: there is no pause, and no "blocked" at the agent level.
enum AgentState: String, CaseIterable, Hashable {
    case working
    case waitingApproval
    case idle
    case failed
    case unknown

    init(agent: Agent, pendingApprovals: Int) {
        if pendingApprovals > 0 || agent.status == "waiting" {
            self = .waitingApproval
            return
        }
        switch agent.status {
        case "running": self = .working
        case "error": self = agent.running == true ? .working : .failed
        case "idle": self = agent.running == true ? .working : .idle
        default: self = .unknown
        }
    }

    var label: String {
        switch self {
        case .working: "Working"
        case .waitingApproval: "Needs approval"
        case .idle: "Idle"
        case .failed: "Last run failed"
        case .unknown: "Unknown"
        }
    }

    var shortLabel: String {
        switch self {
        case .working: "Working"
        case .waitingApproval: "Approval"
        case .idle: "Idle"
        case .failed: "Failed"
        case .unknown: "Unknown"
        }
    }

    var symbol: String {
        switch self {
        case .working: "bolt.fill"
        case .waitingApproval: "hand.raised.fill"
        case .idle: "moon.zzz.fill"
        case .failed: "exclamationmark.triangle.fill"
        case .unknown: "questionmark.circle"
        }
    }

    var needsAttention: Bool { self == .waitingApproval || self == .failed }

    /// Sort order: what needs you first, then active work.
    var rank: Int {
        switch self {
        case .waitingApproval: 0
        case .failed: 1
        case .working: 2
        case .idle: 3
        case .unknown: 4
        }
    }
}

/// Task status as stored by the backend. Unrecognized values map to `.unknown`
/// instead of being guessed.
enum TaskState: String, CaseIterable, Hashable, Identifiable {
    case inbox
    case ready
    case inProgress = "in_progress"
    case blocked
    case waitingApproval = "waiting_approval"
    case done
    case failed
    case cancelled
    case unknown

    var id: String { rawValue }

    init(raw: String) { self = TaskState(rawValue: raw) ?? .unknown }

    var label: String {
        switch self {
        case .inbox: "Inbox"
        case .ready: "Ready"
        case .inProgress: "In progress"
        case .blocked: "Blocked"
        case .waitingApproval: "Waiting for approval"
        case .done: "Done"
        case .failed: "Failed"
        case .cancelled: "Cancelled"
        case .unknown: "Unknown"
        }
    }

    var symbol: String {
        switch self {
        case .inbox: "tray"
        case .ready: "circle"
        case .inProgress: "circle.dotted.circle"
        case .blocked: "pause.circle"
        case .waitingApproval: "hand.raised.circle"
        case .done: "checkmark.circle.fill"
        case .failed: "xmark.circle.fill"
        case .cancelled: "minus.circle"
        case .unknown: "questionmark.circle"
        }
    }

    /// An agent owns the task right now; the user cancels rather than edits status.
    var isActive: Bool { self == .inProgress || self == .blocked || self == .waitingApproval }
    var isClosed: Bool { self == .done || self == .cancelled }
    var needsAttention: Bool { self == .failed || self == .blocked || self == .waitingApproval }
    /// Can be handed to an agent (again).
    var canExecute: Bool { self == .inbox || self == .ready || self == .failed || self == .unknown }
}

enum TaskPriority: String, CaseIterable, Identifiable {
    case low, normal, high, urgent
    var id: String { rawValue }
    var label: String { rawValue.capitalized }
    var rank: Int { [.urgent: 0, .high: 1, .normal: 2, .low: 3][self] ?? 2 }
    var symbol: String? {
        switch self {
        case .urgent: "exclamationmark.3"
        case .high: "exclamationmark.2"
        case .normal, .low: nil
        }
    }
}

/// The task-list tabs.
enum TaskFilter: String, CaseIterable, Identifiable {
    case all, inbox, ready, inProgress, blocked, done, failed
    var id: String { rawValue }

    var label: String {
        switch self {
        case .all: "All"
        case .inbox: "Inbox"
        case .ready: "Ready"
        case .inProgress: "In progress"
        case .blocked: "Blocked"
        case .done: "Done"
        case .failed: "Failed"
        }
    }

    func matches(_ s: TaskState) -> Bool {
        switch self {
        case .all: s != .cancelled
        case .inbox: s == .inbox
        case .ready: s == .ready
        case .inProgress: s == .inProgress
        case .blocked: s == .blocked || s == .waitingApproval
        case .done: s == .done || s == .cancelled
        case .failed: s == .failed
        }
    }

    var emptyMessage: String {
        switch self {
        case .all: "No tasks yet. Add one with the + button."
        case .inbox: "Your inbox is empty."
        case .ready: "Nothing is queued up for an agent."
        case .inProgress: "No agent is working on a task right now."
        case .blocked: "Nothing is blocked."
        case .done: "No finished tasks yet."
        case .failed: "No failed tasks."
        }
    }
}
