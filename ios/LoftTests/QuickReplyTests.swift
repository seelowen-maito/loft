import XCTest
@testable import Loft

/// The rules around the quick acknowledgement and the route guess (no model needed).
final class QuickReplyTests: XCTestCase {
    func testTheAcknowledgementIsTheAppsSentence() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("iOS 26") }
        XCTAssertEqual(OnDeviceJobs.ack(subject: "why the backup numbers look wrong.", language: "en"),
                       "Got it, looking into why the backup numbers look wrong.")
        XCTAssertEqual(OnDeviceJobs.ack(subject: "a tela de login do Vaultly", language: "pt"), "Entendi, vou ver a tela de login do Vaultly.")
    }

    func testTheSubjectMustComeFromTheMessage() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("iOS 26") }
        let msg = "Por que os números do backup do Mac mini estão errados?"
        XCTAssertNil(OnDeviceJobs.problem(withSubject: "números do backup do Mac mini", message: msg))
        XCTAssertNotNil(OnDeviceJobs.problem(withSubject: "Backup numbers Mac mini incorrect", message: msg), "translated: not used")
        XCTAssertNotNil(OnDeviceJobs.problem(withSubject: "backup", message: msg), "too short")
    }

    func testItNeverClaimsSomethingHappened() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("iOS 26") }
        let msg = "Check the deploy of the docs site"
        XCTAssertNotNil(OnDeviceJobs.problem(withSubject: "docs site deployed fixed", message: msg))
        // Words the message itself used are fine ("merge the approved PRs").
        XCTAssertNil(OnDeviceJobs.problem(withSubject: "merge the approved Vaultly PRs", message: "Merge the approved Vaultly PRs"))
    }

    func testRoutesOnlyToAgentsThatExist() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("iOS 26") }
        let agents: [OnDeviceJobs.RouteTarget] = ["Shop Dev", "Shop mobile dev", "Shop Tester", "Shop Mobile QA",
                                                  "Vault Dev", "Vault Security"].map { .init(name: $0, project: nil) }
        XCTAssertEqual(OnDeviceJobs.projects(of: agents), ["shop", "vault"], "projects come from the agents' names")
        func to(_ p: String, _ r: OnDeviceJobs.Role) -> String? { OnDeviceJobs.agent(for: .init(project: p, role: r), agents: agents) }
        XCTAssertEqual(to("shop", .backend), "Shop Dev")
        XCTAssertEqual(to("shop", .iphone), "Shop mobile dev")
        XCTAssertEqual(to("shop", .testing), "Shop Tester")
        XCTAssertEqual(to("vault", .security), "Vault Security")
        XCTAssertNil(to("vault", .marketing), "no such agent: no suggestion")
        XCTAssertNil(to("shop", .overview), "the Orchestrator's own job")
        XCTAssertNil(to(OnDeviceJobs.Route.unclear, .backend))
        // The fixture's names ("iOS Agent") with a project field; a role word ("QA Agent") never becomes a project.
        let fixture: [OnDeviceJobs.RouteTarget] = [.init(name: "Backend Agent", project: "loft"), .init(name: "iOS Agent", project: "loft"),
                                                   .init(name: "QA Agent", project: nil)]
        XCTAssertEqual(OnDeviceJobs.projects(of: fixture), ["loft"])
        XCTAssertEqual(OnDeviceJobs.agent(for: .init(project: "loft", role: .iphone), agents: fixture), "iOS Agent")
        XCTAssertEqual(OnDeviceJobs.agent(for: .init(project: "loft", role: .backend), agents: fixture), "Backend Agent")
    }
}
