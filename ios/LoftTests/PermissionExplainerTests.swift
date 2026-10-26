import XCTest
@testable import Loft

final class PermissionExplainerTests: XCTestCase {
    /// 32 requests like the ones agents make (the same set as ios/Evaluations/cases/explain.json).
    private let cases: [(tool: String, key: String, value: String, risks: [PermissionRisk])] = [
        ("Bash", "command", "rm -rf build/ DerivedData/", [.deletes]),
        ("Bash", "command", "git push origin main", [.network, .pushesToMain]),
        ("Bash", "command", "git push --force origin feature/login", [.deletes, .network]),
        ("Bash", "command", "xcodebuild test -scheme Loft", []),
        ("Bash", "command", "cat .env", [.secrets]),
        ("Bash", "command", "security find-generic-password -s loft -w", [.secrets]),
        ("Bash", "command", "curl -X POST https://api.stripe.com/v1/charges -d amount=500", [.spendsMoney, .network]),
        ("Bash", "command", "osascript -e 'tell application \"Messages\" to send \"done\" to buddy \"+15550100100\"'", [.sendsMessages]),
        ("Bash", "command", "gh pr merge 108 --squash", [.network, .pushesToMain]),
        ("Bash", "command", "gh pr comment 97 --body \"Finding 39 must not merge\"", [.sendsMessages, .network]),
        ("Bash", "command", "npm install", [.network]),
        ("Bash", "command", "ls -la ~/dev", []),
        ("Bash", "command", "git status", []),
        ("Bash", "command", "git branch -D old-feature", [.deletes]),
        ("Bash", "command", "psql -c 'DROP TABLE sessions'", [.deletes]),
        ("Bash", "command", "terraform apply -auto-approve", [.spendsMoney]),
        ("Bash", "command", "ssh deploy@staging.example.com 'systemctl restart api'", [.network]),
        ("Bash", "command", "printenv", [.secrets]),
        ("Bash", "command", "swift build -c release", []),
        ("Bash", "command", "cp ~/.ssh/id_ed25519 /tmp/key", [.secrets]),
        ("Bash", "command", "git clean -fdx", [.deletes]),
        ("Bash", "command", "npm publish", [.spendsMoney, .network]),
        ("Bash", "command", "brew install restic", [.network]),
        ("Write", "file_path", "/Users/sam/dev/loft/README.md", [.deletes]),
        ("Edit", "file_path", "/Users/sam/dev/loft/ios/Loft/App/LoftStore.swift", []),
        ("Read", "file_path", "/Users/sam/dev/api/.env.production", [.secrets]),
        ("WebFetch", "url", "https://developer.apple.com/documentation/foundationmodels", [.network]),
        ("Bash", "command", "git push origin fix/login-keyboard", [.network]),
        ("Bash", "command", "rm ~/Library/Application\\ Support/Loft/apns/AuthKey.p8", [.deletes, .secrets]),
        ("Bash", "command", "aws configure list", [.secrets]),
        ("Bash", "command", "git reset --hard origin/main", [.deletes]), // origin/main is the local copy: no network
        ("Bash", "command", "python3 scripts/send_email.py --to team@example.com", [.sendsMessages]),    ]

    func testTheRisksComeFromTheRequest() {
        for c in cases {
            let got = PermissionRisk.of(tool: c.tool, input: .object([c.key: .string(c.value)]))
            XCTAssertEqual(Set(got), Set(c.risks), "\(c.tool): \(c.value)")
        }
    }

    func testAnExplanationThatSoundsReassuringIsNeverShown() {
        let req = "Bash rm -rf build/"
        XCTAssertNil(PermissionExplainer.problem("Deletes the build folder and everything in it.", request: req))
        for s in ["This is safe: it deletes the build folder.", "It just removes some build files.", "Harmless cleanup of the build folder.",
                  "Isso é seguro, apaga a pasta build.", "Apenas apaga a pasta build."] {
            XCTAssertEqual(PermissionExplainer.problem(s, request: req), "sounds reassuring", s)
        }
    }

    func testAnExplanationCantNameAnotherPathOrNumber() {
        XCTAssertNotNil(PermissionExplainer.problem("Deletes the files in /Users/sam/dev and its subfolders.", request: "Bash rm -rf build/"))
        XCTAssertNotNil(PermissionExplainer.problem("Merges pull request 109 into the main branch.", request: "Bash gh pr merge 108 --squash"))
        XCTAssertNil(PermissionExplainer.problem("Merges pull request 108 into the main branch.", request: "Bash gh pr merge 108 --squash"))
    }

    func testTheShownSentenceDropsTheToolPrefix() {
        XCTAssertEqual(PermissionExplainer.cleaned("Bash: deletes the build folder.", tool: "Bash"), "Deletes the build folder.")
        XCTAssertEqual(PermissionExplainer.cleaned("This would send your changes to GitHub.", tool: "Bash"), "Send your changes to GitHub.")
    }

    func testASentenceCantUndersellWhatTheCodeFound() {
        let write = PermissionRisk.of(tool: "Write", input: .object(["file_path": .string("/Users/sam/dev/loft/README.md")]))
        XCTAssertNotNil(PermissionExplainer.problem("It reads the contents of the README.md file and displays them.",
                                                    request: "Write /Users/sam/dev/loft/README.md", risks: write))
        XCTAssertNil(PermissionExplainer.problem("It replaces the contents of the README.md file.",
                                                 request: "Write /Users/sam/dev/loft/README.md", risks: write))
    }
}
