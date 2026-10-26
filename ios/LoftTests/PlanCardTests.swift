import XCTest
@testable import Loft

final class PlanCardTests: XCTestCase {
    /// The backend's shape (backend/src/tasks.rs plan_view).
    private let json = #"""
    {"task":{"id":"t1","title":"Settings","status":"in_progress","agent":{"id":"orch","name":"Orchestrator"},"summary":""},
     "subtasks":[
      {"id":"t2","title":"Endpoints","status":"done","agent":{"id":"be","name":"Backend Agent"},"note":"3 of 4","summary":"All 4 in","error":"",
       "updated_at":"2026-10-23T14:00:00Z","started_at":null,"completed_at":null,"depends_on":[],
       "prs":[{"id":"p1","slug":"o/r","number":214,"title":"Endpoints","url":"https://github.com/o/r/pull/214","state":"merged"}]},
      {"id":"t3","title":"Screen","status":"blocked","agent":null,"note":"","summary":"","error":"Waiting for the backend","updated_at":"2026-10-23T14:00:00Z",
       "started_at":null,"completed_at":null,"depends_on":["t2"],"prs":[]}],
     "progress":{"total":2,"done":1,"in_progress":0,"waiting":1,"failed":0,"todo":0,"cancelled":0,"percent":50}}
    """#

    func testTheBackendsPlanDecodes() throws {
        let p = try JSONCoding.decoder.decode(TaskPlan.self, from: Data(json.utf8))
        XCTAssertEqual(p.progress.percent, 50)
        XCTAssertEqual(p.subtasks.first?.prs?.first?.number, 214)
        XCTAssertNil(p.subtasks.last?.agent)
        XCTAssertEqual(p.subtasks.first?.line, "All 4 in", "a done task shows its summary")
        XCTAssertEqual(p.subtasks.last?.line, "Waiting for the backend", "a blocked one shows why")
    }

    func testTheFixtureServesAPlan() async throws {
        let p = try await FixtureAPI().taskPlan(id: "t1")
        XCTAssertEqual(p.task.title, "Implement settings functionality")
        XCTAssertEqual(p.subtasks.count, 4)
        XCTAssertEqual(p.progress.done, 1)
    }

    /// The plan lives in the store, so a card the chat's lazy list recreates shows it at once instead of loading again.
    @MainActor
    func testTheStoreKeepsLoadedPlans() async {
        let store = LoftStore(api: FixtureAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("p-\(UUID()).json")))
        XCTAssertNil(store.plans["t1"])
        await store.loadPlan("t1")
        XCTAssertEqual(store.plans["t1"]?.subtasks.count, 4)
        await store.loadPlan("missing")
        XCTAssertNil(store.plans["missing"], "a plan that can't load leaves nothing behind")
    }
}
