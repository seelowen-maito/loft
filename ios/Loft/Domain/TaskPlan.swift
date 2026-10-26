import Foundation

/// `GET /api/tasks/{id}/plan`: a delegated task and its subtasks as one plan (servers with `plans`).
struct TaskPlan: Codable, Hashable {
    struct AgentRef: Codable, Hashable { var id: String; var name: String }
    struct Parent: Codable, Hashable { var id: String; var title: String; var status: String; var agent: AgentRef?; var summary: String? }
    struct PR: Codable, Hashable { var id: String?; var number: Int?; var title: String?; var url: String?; var state: String? }
    struct Subtask: Codable, Hashable, Identifiable {
        let id: String
        var title: String
        var status: String
        var agent: AgentRef?
        var note: String?
        var summary: String?
        var error: String?
        var updatedAt: Date?
        var prs: [PR]?
        var state: TaskState { TaskState(raw: status) }
        /// The one line worth showing under the title: why it failed or is blocked, else its latest note or summary.
        var line: String? {
            [error, state == .done ? summary : note].compactMap { $0 }.first { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        }
    }
    struct Progress: Codable, Hashable {
        var total: Int; var done: Int; var inProgress: Int; var waiting: Int; var failed: Int; var todo: Int; var cancelled: Int; var percent: Int
    }

    var task: Parent
    var subtasks: [Subtask]
    var progress: Progress
}
