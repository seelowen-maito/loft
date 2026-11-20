import Foundation

/// `GET /api/tasks/{id}/plan`: a delegated task and its subtasks as one plan (servers with `plans`).
public struct TaskPlan: Codable, Hashable {
    public struct AgentRef: Codable, Hashable { public var id: String; public var name: String }
    public struct Parent: Codable, Hashable { public var id: String; public var title: String; public var status: String; public var agent: AgentRef?; public var summary: String? }
    public struct PR: Codable, Hashable { public var id: String?; public var number: Int?; public var title: String?; public var url: String?; public var state: String? }
    public struct Subtask: Codable, Hashable, Identifiable {
        public let id: String
        public var title: String
        public var status: String
        public var agent: AgentRef?
        public var note: String?
        public var summary: String?
        public var error: String?
        public var updatedAt: Date?
        public var prs: [PR]?
        public var state: TaskState { TaskState(raw: status) }
        /// The one line worth showing under the title: why it failed or is blocked, else its latest note or summary.
        public var line: String? {
            [error, state == .done ? summary : note].compactMap { $0 }.first { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        }
    }
    public struct Progress: Codable, Hashable {
        public var total: Int; public var done: Int; public var inProgress: Int; public var waiting: Int; public var failed: Int; public var todo: Int; public var cancelled: Int; public var percent: Int
    }

    public var task: Parent
    public var subtasks: [Subtask]
    public var progress: Progress
}
