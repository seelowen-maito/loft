import XCTest
@testable import Loft

final class StatusTests: XCTestCase {
    private func agent(_ status: String, running: Bool?) -> Agent {
        Agent(id: "a", name: "A", status: status, running: running)
    }

    func testAgentStateMapping() {
        XCTAssertEqual(AgentState(agent: agent("running", running: true), pendingApprovals: 0), .working)
        XCTAssertEqual(AgentState(agent: agent("idle", running: false), pendingApprovals: 0), .idle)
        XCTAssertEqual(AgentState(agent: agent("idle", running: true), pendingApprovals: 0), .working)
        XCTAssertEqual(AgentState(agent: agent("waiting", running: true), pendingApprovals: 0), .waitingApproval)
        XCTAssertEqual(AgentState(agent: agent("running", running: true), pendingApprovals: 1), .waitingApproval)
        XCTAssertEqual(AgentState(agent: agent("error", running: false), pendingApprovals: 0), .failed)
        // A new run after a failure is work, not failure.
        XCTAssertEqual(AgentState(agent: agent("error", running: true), pendingApprovals: 0), .working)
        XCTAssertEqual(AgentState(agent: agent("hibernating", running: nil), pendingApprovals: 0), .unknown)
    }

    func testTaskStates() {
        XCTAssertEqual(TaskState(raw: "in_progress"), .inProgress)
        XCTAssertEqual(TaskState(raw: "paused"), .unknown)
        XCTAssertTrue(TaskState.blocked.isActive)
        XCTAssertFalse(TaskState.failed.isActive)
        XCTAssertTrue(TaskState.failed.canExecute)
        XCTAssertFalse(TaskState.inProgress.canExecute)
        XCTAssertFalse(TaskState.done.canExecute)
        XCTAssertTrue(TaskFilter.blocked.matches(.waitingApproval))
        XCTAssertFalse(TaskFilter.all.matches(.cancelled))
        XCTAssertTrue(TaskFilter.done.matches(.cancelled))
    }

    func testTaskOrderPutsProblemsFirst() {
        func t(_ id: String, _ status: String, _ priority: String = "normal") -> HBTask {
            HBTask(id: id, title: id, description: "", kind: "work", status: status, priority: priority, project: "", agentId: nil,
                   parentId: nil, dependsOn: [], dueAt: nil, createdBy: "user", note: "", summary: "", error: "",
                   createdAt: .now, updatedAt: .now, startedAt: nil, completedAt: nil)
        }
        let sorted = [t("done", "done"), t("low", "inbox", "low"), t("urgent", "inbox", "urgent"), t("run", "in_progress"), t("bad", "failed")]
            .sorted(by: TaskListView.order).map(\.id)
        XCTAssertEqual(sorted, ["bad", "run", "urgent", "low", "done"])
    }

    func testConnectionLabels() {
        XCTAssertEqual(ConnectionState.reconnecting(attempt: 1, error: .timeout).label, "Reconnecting…")
        XCTAssertEqual(ConnectionState.reconnecting(attempt: 3, error: .timeout).label, "Disconnected")
        XCTAssertEqual(ConnectionState.reconnecting(attempt: 3, error: .server(status: 502, message: "")).label, "Server unavailable")
        XCTAssertTrue(ConnectionState.polling.isUsable)
        XCTAssertFalse(ConnectionState.connecting.isUsable)
    }
}

final class RealtimeTests: XCTestCase {
    func testSSEParser() {
        var p = SSEParser()
        XCTAssertNil(p.feed(":connected"))
        XCTAssertNil(p.feed(""))
        XCTAssertNil(p.feed("data: {\"a\":"))
        XCTAssertNil(p.feed("data:1}"))
        XCTAssertNil(p.feed("event: ignored"))
        XCTAssertEqual(p.feed(""), "{\"a\":\n1}")
        XCTAssertNil(p.feed(""))
        XCTAssertNil(p.feed("data: x"))
        XCTAssertEqual(p.feed(""), "x")
    }

    func testBackoffGrowsCapsAndResets() {
        var b = Backoff(base: 1, cap: 8)
        let top = { (r: ClosedRange<Double>) in r.upperBound }
        XCTAssertEqual([b.next(random: top), b.next(random: top), b.next(random: top), b.next(random: top), b.next(random: top)], [1, 2, 4, 8, 8])
        b.reset()
        XCTAssertEqual(b.next(random: { $0.lowerBound }), 0.5)
        // The default ceiling is 10 s, so a Mac that comes back is reconnected to within about 10 s.
        var d = Backoff()
        for _ in 0..<20 { XCTAssertLessThanOrEqual(d.next(), 10) }
        // Jitter stays within half to full of the ceiling.
        var j = Backoff(base: 4, cap: 30)
        for _ in 0..<50 { XCTAssertLessThanOrEqual(j.next(), 30) }
    }

    func testErrorMapping() {
        XCTAssertEqual(APIError.from(URLError(.timedOut)), .timeout)
        XCTAssertEqual(APIError.from(URLError(.serverCertificateUntrusted)), .insecureConnection)
        XCTAssertEqual(APIError.from(URLError(.cancelled)), .cancelled)
        XCTAssertEqual(APIError.from(CancellationError()), .cancelled)
        if case .unreachable = APIError.from(URLError(.cannotFindHost)) {} else { XCTFail() }
        XCTAssertEqual(APIError.server(status: 409, message: "Cancel it first").title, "Cancel it first")
        XCTAssertFalse(APIError.offline.suggestion.isEmpty)
    }
}

final class SpeechMachineTests: XCTestCase {
    func testHappyPath() {
        var s = SpeechState.idle
        s = SpeechMachine.reduce(s, .start); XCTAssertEqual(s, .requestingPermission)
        s = SpeechMachine.reduce(s, .began); XCTAssertEqual(s, .recording)
        XCTAssertTrue(s.isRecording)
        s = SpeechMachine.reduce(s, .finished); XCTAssertEqual(s, .idle)
    }

    func testDenialAndRecovery() {
        var s = SpeechMachine.reduce(.idle, .start)
        s = SpeechMachine.reduce(s, .denied("no mic"))
        XCTAssertEqual(s, .denied("no mic"))
        XCTAssertFalse(s.isRecording)
        XCTAssertEqual(SpeechMachine.reduce(s, .start), .requestingPermission)
    }

    func testStopAndInterruptions() {
        XCTAssertEqual(SpeechMachine.reduce(.recording, .stop), .idle)
        XCTAssertEqual(SpeechMachine.reduce(.requestingPermission, .stop), .idle)
        if case .failed = SpeechMachine.reduce(.recording, .interrupted) {} else { XCTFail() }
        if case .failed = SpeechMachine.reduce(.recording, .error("boom")) {} else { XCTFail() }
        // Late callbacks after a stop don't revive anything.
        XCTAssertEqual(SpeechMachine.reduce(.idle, .began), .idle)
        XCTAssertEqual(SpeechMachine.reduce(.idle, .finished), .idle)
        XCTAssertEqual(SpeechMachine.reduce(.recording, .start), .recording)
    }
}

final class ChatItemTests: XCTestCase {
    private func m(_ id: Int64, _ kind: String, _ role: String = "assistant") -> ChatMessage {
        ChatMessage(id: id, agentId: "a", runId: nil, role: role, kind: kind, content: "c\(id)", meta: nil, createdAt: .now)
    }

    func testGroupsToolCallsAndHidesBookkeeping() {
        let items = ChatItem.build([m(1, "text", "user"), m(2, "run_start", "system"), m(3, "thinking"), m(4, "tool_use"),
                                    m(5, "tool_result"), m(6, "tool_use"), m(7, "text"), m(8, "run_end", "system"), m(9, "clear", "system")])
        XCTAssertEqual(items.map(\.id), ["m1", "t4", "m7", "d9"])
        if case .tools(_, let calls) = items[1] { XCTAssertEqual(calls.map(\.id), [4, 6]) } else { XCTFail() }
    }

    func testToolSearchIsHidden() {
        let search = ChatMessage(id: 2, agentId: "a", runId: nil, role: "assistant", kind: "tool_use", content: "ToolSearch",
                                 meta: ["name": "ToolSearch"], createdAt: .now)
        let items = ChatItem.build([m(1, "text", "user"), search, m(3, "text")])
        XCTAssertEqual(items.map(\.id), ["m1", "m3"], "no tool row for a tool lookup alone")
    }
}

private extension Agent {
    init(id: String, name: String, status: String, running: Bool?) {
        self.init(id: id, name: name, title: nil, project: nil, model: nil, effort: nil, permissionMode: nil, intervalMinutes: nil,
                  status: status, statusText: nil, running: running, lastRunAt: nil, nextRunAt: nil, createdAt: nil,
                  totalCostUsd: nil, contextTokens: nil, prompt: nil)
    }
}
