import XCTest
@testable import LoftKit

/// Saved (spec #675) on the iPhone and Mac: the same rules as the web's frontend/src/lib/saved.ts, the server's JSON,
/// and the demo's list.
final class SavedTests: XCTestCase {
    private func message(_ role: String, _ kind: String, _ content: String) -> ChatMessage {
        ChatMessage(id: 1, agentId: "a", runId: nil, role: role, kind: kind, content: content, meta: nil, createdAt: Date())
    }

    func testOnlyChatTextCanBeSaved() {
        XCTAssertTrue(Saved.canSave(message("assistant", "text", "# Deploy")))
        XCTAssertTrue(Saved.canSave(message("user", "text", "Remember this")))
        XCTAssertFalse(Saved.canSave(message("assistant", "tool_use", "Bash")))
        XCTAssertFalse(Saved.canSave(message("system", "text", "x")))
        XCTAssertFalse(Saved.canSave(message("assistant", "text", "  \n")))
    }

    func testDefaultTitleIsTheFirstLineWithoutMarkdownMarks() {
        XCTAssertEqual(Saved.defaultTitle("\n# How to deploy\n\nRun it."), "How to deploy")
        XCTAssertEqual(Saved.defaultTitle("> a quote"), "a quote")
        XCTAssertEqual(Saved.defaultTitle("- first item\n- second"), "first item")
        XCTAssertEqual(Saved.defaultTitle("1. Install Rust"), "Install Rust")
        XCTAssertEqual(Saved.defaultTitle(""), "")
        // A long line is cut at a word.
        let long = String(repeating: "word ", count: 30)
        XCTAssertEqual(Saved.defaultTitle(long), String(repeating: "word ", count: 16).trimmingCharacters(in: .whitespaces) + "…")
        XCTAssertEqual(Saved.defaultTitle(String(repeating: "x", count: 90)), String(repeating: "x", count: 80) + "…")
    }

    func testTagOptionsAreTheSuggestedThenTheOthersOnceEach() {
        XCTAssertEqual(Saved.tagOptions(["decision", "Recipes", " ", "recipes"]), ["Tutorial", "Good to know", "Decision", "Recipes"])
    }

    func testExcerptIsPlainWords() {
        XCTAssertEqual(Saved.excerpt("# Title\n\n| a | b |\n|---|---|\n| **1** | `2` |"), "Title a b 1 2")
        XCTAssertEqual(Saved.excerpt(String(repeating: "a", count: 200), chars: 10), "aaaaaaaaaa…")
        XCTAssertEqual(Saved.excerpt("Step | Time\n--- | :---:\nbuild | 2\n\n---\nDone"), "Step Time build 2 Done")
    }

    func testExcerptOfALongLineOfSpacesIsFast() {
        let start = Date()
        XCTAssertEqual(Saved.excerpt(String(repeating: " ", count: 100_000) + "x"), "x")
        XCTAssertLessThan(Date().timeIntervalSince(start), 1)
    }

    func testDecodesTheServersItem() throws {
        let json = """
        {"id": 7, "title": "Deploy", "tag": "Tutorial", "content": "Run it", "agent_id": "a1", "agent_name": "Dev",
         "message_id": 42, "created_at": "2026-11-23T10:00:00Z", "updated_at": null, "source_exists": false}
        """
        let i = try JSONCoding.decoder.decode(SavedItem.self, from: Data(json.utf8))
        XCTAssertEqual(i.id, 7)
        XCTAssertEqual(i.agentName, "Dev")
        XCTAssertEqual(i.messageId, 42)
        XCTAssertNil(i.updatedAt)
        XCTAssertEqual(i.sourceExists, false)
        let list = try JSONCoding.decoder.decode(SavedList.self, from: Data(#"{"items": [], "tags": ["Tutorial"]}"#.utf8))
        XCTAssertEqual(list.tags, ["Tutorial"])
    }

    func testTheDemoSavesSearchesEditsAndDeletes() async throws {
        let api = FixtureAPI()
        let saved = try await api.saveMessage(agentId: "orch", messageId: 2, title: "", tag: "Decision")
        XCTAssertEqual(saved.title, "On it. I'll split this into backend, iOS and QA sub-issues.")
        var list = try await api.savedList(query: "", tag: "")
        XCTAssertEqual(list.items.first?.id, saved.id, "newest first")
        XCTAssertEqual(list.tags, ["Decision", "Good to know", "Tutorial"])
        list = try await api.savedList(query: "CARGO", tag: "")
        XCTAssertEqual(list.items.map(\.title), ["How to run the backend locally"])
        list = try await api.savedList(query: "", tag: "decision")
        XCTAssertEqual(list.items.map(\.id), [saved.id])

        let item = try await api.savedItem(id: saved.id)
        XCTAssertEqual(item.sourceExists, true)
        let gone = try await api.savedItem(id: 1)
        XCTAssertEqual(gone.sourceExists, false, "the tutorial's message isn't in the chat")

        let edited = try await api.updateSaved(id: saved.id, title: "Split", tag: "Tutorial", content: "Three issues")
        XCTAssertEqual(edited.title, "Split")
        XCTAssertNotNil(edited.updatedAt)
        try await api.deleteSaved(id: saved.id)
        list = try await api.savedList(query: "", tag: "")
        XCTAssertFalse(list.items.contains { $0.id == saved.id })

        // A tool call isn't chat text.
        do {
            _ = try await api.saveMessage(agentId: "orch", messageId: 3, title: "t", tag: "")
            XCTFail("a tool call can't be saved")
        } catch {}
    }
}
