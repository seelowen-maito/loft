import XCTest
@testable import Loft

final class TasksInChatsTests: XCTestCase {
    func testTaskLinks() {
        XCTAssertEqual(TaskLink.id(from: URL(string: "loft://task/ABC-123")!), "abc-123")
        XCTAssertEqual(TaskLink.id(from: URL(string: "http://my-mac.tail1234.ts.net:7420/tasks/t42")!), "t42")
        XCTAssertNil(TaskLink.id(from: URL(string: "loft://task/")!))
        XCTAssertNil(TaskLink.id(from: URL(string: "https://github.com/Octo/loft/pull/116")!))
        XCTAssertEqual(TaskLink.url("t1").absoluteString, "loft://task/t1")
    }

    private func message(_ meta: JSONValue?, content: String = "hi") -> ChatMessage {
        ChatMessage(id: 1, agentId: "orch", runId: nil, role: "user", kind: "text", content: content, meta: meta, createdAt: Date())
    }

    func testChipsComeFromTaskRefsAttachedFirst() {
        XCTAssertEqual(message(.object(["task_id": .string("a"), "task_refs": .array([.string("a"), .string("b")])])).taskRefs, ["a", "b"])
        XCTAssertEqual(message(.object(["task_id": .string("a")])).taskRefs, ["a"], "an older server: just the task id")
        XCTAssertEqual(message(nil).taskRefs, [])
    }

    func testThePickerPutsWhatWaitsOnYouFirst() {
        let now = Date()
        func t(_ id: String, _ status: String, _ minutesAgo: Double, project: String = "loft") -> HBTask {
            HBTask(id: id, title: "Task \(id)", description: "", kind: "work", status: status, priority: "normal", project: project, agentId: nil,
                   parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "", createdAt: now,
                   updatedAt: now.addingTimeInterval(-minutesAgo * 60), startedAt: nil, completedAt: nil)
        }
        let tasks = [t("done", "done", 1), t("old", "in_progress", 60), t("new", "in_progress", 5), t("ask", "waiting_approval", 90),
                     t("api", "inbox", 3, project: "api")]
        XCTAssertEqual(TaskPicker.list(tasks, query: "").map(\.id), ["ask", "new", "old", "api", "done"])
        XCTAssertEqual(TaskPicker.list(tasks, query: "API").map(\.id), ["api"], "searches the project too")
    }

    func testTheFixtureStoresAnAttachedTaskAndRejectsAnUnknownOne() async throws {
        let api = FixtureAPI()
        _ = try await api.sendMessage(agentId: "orch", text: "look at this", clientId: "c1", meta: .object(["task_id": .string("t2")]))
        let last = try await api.messages(agentId: "orch", limit: 100, before: nil).last
        XCTAssertEqual(last?.taskRefs, ["t2"])
        do {
            _ = try await api.sendMessage(agentId: "orch", text: "x", clientId: "c2", meta: .object(["task_id": .string("nope")]))
            XCTFail("an unknown task is a 400")
        } catch {}
    }
}
