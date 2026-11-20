import XCTest
@testable import Loft
@testable import LoftKit

/// Payloads copied from a real Loft backend.
final class DecodingTests: XCTestCase {
    let agentJSON = """
    {"id":"4be555ea-7566-4b3e-95d2-5a8f323bbbaf","name":"Web Dev","title":"","project":"loft","personality":"",
     "prompt":"Build things","model":"opus","effort":"","interval_minutes":60,"quiet_hours":[],"allowed_tools":["Read"],
     "disallowed_tools":[],"mcp_servers":{},"permission_mode":"auto","subagent_model":"","max_subagents":null,
     "avatar":{"svg":"<svg/>"},"status":"running","status_text":"Thinking…","session_id":"abc","context_tokens":12000,
     "last_run_at":"2026-10-21T13:01:02.123456+00:00","next_run_at":null,"total_cost_usd":1.5,
     "created_at":"2026-10-01T09:00:00+00:00","running":true,"workspace":"/tmp/x",
     "fresh_context":"above_80k","fresh_context_applies":false}
    """

    func testAgent() throws {
        let a = try JSONCoding.decoder.decode(Agent.self, from: Data(agentJSON.utf8))
        XCTAssertEqual(a.name, "Web Dev")
        XCTAssertEqual(a.statusText, "Thinking…")
        XCTAssertEqual(a.intervalMinutes, 60)
        XCTAssertEqual(a.running, true)
        XCTAssertEqual(a.contextTokens, 12000)
        XCTAssertEqual(a.sessionId, "abc")
        XCTAssertNotNil(a.lastRunAt)
        XCTAssertNil(a.nextRunAt)
        XCTAssertEqual(a.freshContext, "above_80k")
        XCTAssertEqual(a.freshContextApplies, false)
    }

    /// Agents have no Title: a server that still sends one decodes, and so does one that doesn't; the Role is its label.
    func testAgentWithOrWithoutTitle() throws {
        let titled = agentJSON.replacingOccurrences(of: #""title":"","#, with: #""title":"Builds the web","role":"dev","#)
        XCTAssertEqual(try JSONCoding.decoder.decode(Agent.self, from: Data(titled.utf8)).roleLabel, "Dev")
        let untitled = agentJSON.replacingOccurrences(of: #""title":"","#, with: #""role":"qa","#)
        XCTAssertEqual(try JSONCoding.decoder.decode(Agent.self, from: Data(untitled.utf8)).roleLabel, "QA")
    }

    /// An agent's group and projects (main first); an older server sends only `project`, which then is its one project; with no group it has no team.
    func testAgentGroupAndProjects() throws {
        let with = agentJSON.replacingOccurrences(of: #""project":"loft","#, with: #""project":"api","group":" Dineout ","projects":["api","pos"],"#)
        let a = try JSONCoding.decoder.decode(Agent.self, from: Data(with.utf8))
        XCTAssertEqual(a.allProjects, ["api", "pos"])
        XCTAssertEqual(a.team, "Dineout")
        let old = try JSONCoding.decoder.decode(Agent.self, from: Data(agentJSON.utf8))
        XCTAssertNil(old.group)
        XCTAssertEqual(old.allProjects, ["loft"])
        XCTAssertEqual(old.team, "")
        let empty = agentJSON.replacingOccurrences(of: #""project":"loft","#, with: #""project":"","group":"","projects":[],"#)
        let e = try JSONCoding.decoder.decode(Agent.self, from: Data(empty.utf8))
        XCTAssertEqual(e.allProjects, [])
        XCTAssertEqual(e.team, "")
    }

    /// `/pull-requests`: the verdict labels come with the PR; an older server sends none.
    func testPullRequestLabels() throws {
        let base = #"{"id":"p","agent_id":"a","slug":"o/r","number":7,"url":"u","title":"T","state":"open","draft":true"#
        let with = try JSONCoding.decoder.decode(PullRequest.self, from: Data((base + #","labels":["tests:green","security:passed"]}"#).utf8))
        XCTAssertEqual(with.labels, ["tests:green", "security:passed"])
        let without = try JSONCoding.decoder.decode(PullRequest.self, from: Data((base + "}").utf8))
        XCTAssertNil(without.labels)
    }

    /// `/agents/{id}/sessions`: the start time is Claude Code's transcript timestamp.
    func testPastSessions() throws {
        let json = """
        {"sessions":[{"id":"s2","started_at":"2026-10-21T13:01:02.123Z","first_message":"Check the login flow",
         "from_agent":"Loft Lead","context_tokens":48000},
         {"id":"s1","started_at":"2026-10-20T09:00:00.000Z","first_message":"Hi","from_agent":null,"context_tokens":null}],
         "current":"s2"}
        """
        let p = try JSONCoding.decoder.decode(PastSessions.self, from: Data(json.utf8))
        XCTAssertEqual(p.sessions.map(\.id), ["s2", "s1"])
        XCTAssertEqual(p.current, "s2")
        XCTAssertEqual(p.sessions[0].fromAgent, "Loft Lead")
        XCTAssertEqual(p.sessions[0].contextTokens, 48000)
        XCTAssertEqual(p.sessions[0].startedAt, JSONCoding.parseDate("2026-10-21T13:01:02.123Z"))
        XCTAssertNil(p.sessions[1].contextTokens)
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

    /// A task from a Loft with issue fields: `labels`, `owner` and an open/closed `state` next to `status`.
    /// The app reads the execution status from `status`; the extra keys must not get in the way.
    func testTaskWithIssueFields() throws {
        let json = """
        {"agent_id":null,"completed_at":null,"created_at":"2026-10-21T13:06:33.526952+00:00","created_by":"user","depends_on":[],
         "description":"d","due_at":null,"error":"","id":"6f1d","kind":"todo","note":"","parent_id":null,
         "labels":["bug","ready-for-agent"],"owner":"octo-dev","state":"open",
         "priority":"normal","project":"loft","started_at":null,"status":"in_progress","summary":"","title":"Ship",
         "updated_at":"2026-10-21T13:06:33.526952+00:00"}
        """
        let t = try JSONCoding.decoder.decode(HBTask.self, from: Data(json.utf8))
        XCTAssertEqual(t.state, .inProgress)
        XCTAssertEqual(t.title, "Ship")
        let closed = try JSONCoding.decoder.decode(HBTask.self, from: Data(json.replacingOccurrences(of: #""status":"in_progress""#, with: #""status":"done""#).replacingOccurrences(of: #""state":"open""#, with: #""state":"closed""#).utf8))
        XCTAssertEqual(closed.state, .done)
    }

    /// A mirrored Loft issue (spec #65): labels, owner and its GitHub link with sub-issue counts.
    func testTaskMirroringAGitHubIssue() throws {
        let json = """
        {"agent_id":null,"completed_at":null,"created_at":"2026-10-21T13:06:33Z","created_by":"user","depends_on":[],
         "description":"d","due_at":null,"error":"","id":"6f1d","kind":"work","note":"","parent_id":null,
         "labels":["spec"],"owner":"octo-dev","state":"open",
         "github":{"repo":"octo/loft","number":42,"id":991,"url":"https://github.com/octo/loft/issues/42","state":"open","updated_at":"2026-10-21T13:00:00Z","sub_total":4,"sub_done":1},
         "priority":"normal","project":"loft","started_at":null,"status":"ready","summary":"","title":"Ship",
         "updated_at":"2026-10-21T13:06:33Z"}
        """
        let t = try JSONCoding.decoder.decode(HBTask.self, from: Data(json.utf8))
        XCTAssertEqual(t.github?.shortName, "loft#42")
        XCTAssertEqual(t.github?.subIssues, "1/4 sub-issues")
        XCTAssertEqual(t.ownerLogin, "octo-dev")
        XCTAssertEqual(t.labels, ["spec"])
        // A Loft-only issue, and an older server without the fields.
        let plain = try JSONCoding.decoder.decode(HBTask.self, from: Data(json.replacingOccurrences(of: #""github":{"repo":"octo/loft","number":42,"id":991,"url":"https://github.com/octo/loft/issues/42","state":"open","updated_at":"2026-10-21T13:00:00Z","sub_total":4,"sub_done":1},"#, with: #""github":null,"#).replacingOccurrences(of: #""owner":"octo-dev""#, with: #""owner":"""#).utf8))
        XCTAssertNil(plain.github)
        XCTAssertNil(plain.ownerLogin, "an empty owner is no owner")
        XCTAssertNil(HBTask.GitHubLink(repo: "o/r", number: 1, url: "", state: "open", subTotal: 0).subIssues)
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
