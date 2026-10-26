import XCTest
@testable import Loft

/// Payloads copied from a real Loft backend.
final class DecodingTests: XCTestCase {
    let agentJSON = """
    {"id":"4be555ea-7566-4b3e-95d2-5a8f323bbbaf","name":"Web Dev","title":"","project":"loft","personality":"",
     "prompt":"Build things","model":"opus","effort":"","interval_minutes":60,"quiet_hours":[],"allowed_tools":["Read"],
     "disallowed_tools":[],"mcp_servers":{},"permission_mode":"auto","subagent_model":"","max_subagents":null,
     "avatar":{"svg":"<svg/>"},"status":"running","status_text":"Thinking…","session_id":"abc","context_tokens":12000,
     "last_run_at":"2026-10-21T13:01:02.123456+00:00","next_run_at":null,"total_cost_usd":1.5,
     "created_at":"2026-10-01T09:00:00+00:00","running":true,"workspace":"/tmp/x"}
    """

    func testAgent() throws {
        let a = try JSONCoding.decoder.decode(Agent.self, from: Data(agentJSON.utf8))
        XCTAssertEqual(a.name, "Web Dev")
        XCTAssertEqual(a.statusText, "Thinking…")
        XCTAssertEqual(a.intervalMinutes, 60)
        XCTAssertEqual(a.running, true)
        XCTAssertEqual(a.contextTokens, 12000)
        XCTAssertNotNil(a.lastRunAt)
        XCTAssertNil(a.nextRunAt)
    }

    func testTask() throws {
        let json = """
        {"agent_id":null,"completed_at":null,"created_at":"2026-10-21T13:06:33.526952+00:00","created_by":"user","depends_on":[],
         "description":"d","due_at":"2026-10-24T09:00:00+00:00","error":"","id":"6f1c","kind":"todo","note":"","parent_id":null,
         "priority":"high","project":"loft","started_at":null,"status":"waiting_approval","summary":"","title":"Ship",
         "updated_at":"2026-10-21T13:06:33.526952+00:00"}
        """
        let t = try JSONCoding.decoder.decode(HBTask.self, from: Data(json.utf8))
        XCTAssertEqual(t.state, .waitingApproval)
        XCTAssertEqual(t.priorityLevel, .high)
        XCTAssertTrue(t.isPersonalTodo)
        XCTAssertFalse(t.isDelegated)
        XCTAssertEqual(t.dueAt, JSONCoding.parseDate("2026-10-24T09:00:00Z"))
    }

    func testMessageMeta() throws {
        let json = """
        {"id":42,"agent_id":"a","run_id":null,"role":"user","kind":"text","content":"[Task 123e4567-e89b-12d3-a456-426614174000] Do it\\n\\nDetails\\n\\n[Report progress on this task with x]",
         "meta":{"from_agent":{"id":"o","name":"Orchestrator"},"client_id":"c1"},"created_at":"2026-10-21T13:00:00.5+00:00"}
        """
        let m = try JSONCoding.decoder.decode(ChatMessage.self, from: Data(json.utf8))
        XCTAssertEqual(m.fromAgentName, "Orchestrator")
        XCTAssertEqual(m.fromAgentId, "o")
        XCTAssertEqual(m.clientId, "c1")
        XCTAssertEqual(m.referencedTaskId, "123e4567-e89b-12d3-a456-426614174000")
        XCTAssertEqual(m.displayContent, "Do it\n\nDetails")
    }

    func testDisplayContentStripsInlineTaskTags() throws {
        let json = """
        {"id":7,"agent_id":"h","run_id":null,"role":"user","kind":"text",
         "content":"[Task 4eb34d41-ef96-4fed-9f14-88922d339815] Write a haiku\\n\\nThis is a subtask of [Task 39990bc6-4827-4e30-95c5-c241181d1942] Write a haiku about Loft.",
         "meta":{"from_agent":{"id":"o","name":"Orchestrator"},"task_id":"4eb34d41-ef96-4fed-9f14-88922d339815","task_role":"brief"},
         "created_at":"2026-10-21T13:21:00+00:00"}
        """
        let m = try JSONCoding.decoder.decode(ChatMessage.self, from: Data(json.utf8))
        XCTAssertEqual(m.displayContent, "Write a haiku\n\nThis is a subtask of Write a haiku about Loft.")
        XCTAssertTrue(m.isTaskBrief)
        XCTAssertEqual(m.referencedTaskId, "4eb34d41-ef96-4fed-9f14-88922d339815")
    }

    func testDisplayContentLeavesOrdinaryText() {
        XCTAssertEqual(ChatMessage.splitTaskTag("[Task not-a-uuid] hi").id, nil)
        XCTAssertEqual(ChatMessage.splitTaskTag("hello").rest, "hello")
    }

    func testEvents() throws {
        func ev(_ s: String) throws -> ServerEvent { try ServerEvent.decode(Data(s.utf8)) }
        guard case .agentUpdated(let a) = try ev(#"{"type":"agent_updated","agent":\#(agentJSON)}"#) else { return XCTFail() }
        XCTAssertEqual(a.status, "running")
        guard case .agentDeleted("x") = try ev(#"{"type":"agent_deleted","agent_id":"x"}"#) else { return XCTFail() }
        guard case .taskDeleted("t") = try ev(#"{"type":"task_deleted","task_id":"t"}"#) else { return XCTFail() }
        guard case .other("memory") = try ev(#"{"type":"memory","agent_id":"x"}"#) else { return XCTFail() }
        let perm = #"{"type":"permission","request":{"id":"p","agent_id":"a","source":"prompt","tool_name":"Bash","input":{"command":"rm -rf build"},"reason":"","rule":"Bash(rm *)","status":"pending","created_at":"2026-10-21T13:00:00+00:00"}}"#
        guard case .permission(let p) = try ev(perm) else { return XCTFail() }
        XCTAssertTrue(p.isPending)
        XCTAssertEqual(p.summary, "Bash: rm -rf build")
        XCTAssertThrowsError(try ev(#"{"type":"message"}"#))
    }

    func testDates() {
        let base = JSONCoding.parseDate("2026-10-21T13:06:33Z")!
        XCTAssertEqual(JSONCoding.parseDate("2026-10-21T13:06:33+00:00"), base)
        XCTAssertEqual(JSONCoding.parseDate("2026-10-21T13:06:33.5+00:00")!.timeIntervalSince(base), 0.5, accuracy: 0.001)
        XCTAssertEqual(JSONCoding.parseDate("2026-10-21T13:06:33.526952+00:00")!.timeIntervalSince(base), 0.526, accuracy: 0.001)
        XCTAssertEqual(JSONCoding.parseDate("2026-10-21T15:06:33.123+02:00")!.timeIntervalSince(base), 0.123, accuracy: 0.001)
        XCTAssertNil(JSONCoding.parseDate("yesterday"))
        let d = Date(timeIntervalSince1970: 1_800_000_000.25)
        XCTAssertEqual(JSONCoding.parseDate(JSONCoding.formatDate(d))!.timeIntervalSince1970, d.timeIntervalSince1970, accuracy: 0.001)
    }

    func testURLNormalization() {
        XCTAssertEqual(LiveLoftAPI.normalize("my-mac.tail.ts.net")?.absoluteString, "https://my-mac.tail.ts.net")
        XCTAssertEqual(LiveLoftAPI.normalize(" https://my-mac.tail.ts.net/api/ ")?.absoluteString, "https://my-mac.tail.ts.net")
        XCTAssertEqual(LiveLoftAPI.normalize("http://127.0.0.1:7430")?.absoluteString, "http://127.0.0.1:7430")
        XCTAssertNil(LiveLoftAPI.normalize(""))
        XCTAssertNil(LiveLoftAPI.normalize("ftp://x"))
    }
}
