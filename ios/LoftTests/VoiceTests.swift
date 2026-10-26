import AVFoundation
import Speech
import XCTest
@testable import Loft

final class VoiceTextTests: XCTestCase {
    func testEndPhrases() {
        XCTAssertEqual(VoiceText.strippingEndPhrase("Fix the login bug. I'm done."), "Fix the login bug.")
        XCTAssertEqual(VoiceText.strippingEndPhrase("fix the login bug im done"), "fix the login bug")
        XCTAssertEqual(VoiceText.strippingEndPhrase("Roda os testes de novo, terminei"), "Roda os testes de novo")
        XCTAssertEqual(VoiceText.strippingEndPhrase("Olha os logs. É isso."), "Olha os logs.")
        XCTAssertEqual(VoiceText.strippingEndPhrase("That's it"), "")
        // Only at the end, and only whole words.
        XCTAssertNil(VoiceText.strippingEndPhrase("I'm done with the backend, now look at iOS"))
        XCTAssertNil(VoiceText.strippingEndPhrase("the build is prontomente broken"))
        XCTAssertNil(VoiceText.strippingEndPhrase(""))
    }

    func testJoin() {
        XCTAssertEqual(VoiceText.join(["First part.", "  ", "second part ", ""]), "First part. second part")
    }

    func testSpeakableSkipsCodeAndMarkdown() {
        let md = """
        ## Done
        I fixed **two** bugs in `api.rs`:

        - The [login](https://github.com/x/y/pull/1) flow
        - Settings crash

        ```rust
        fn main() {}
        ```

        | a | b |
        |---|---|
        See https://example.com for details.
        """
        let (text, truncated) = VoiceText.speakable(md)
        XCTAssertFalse(truncated)
        XCTAssertFalse(text.contains("fn main"))
        XCTAssertFalse(text.contains("**") || text.contains("`") || text.contains("##") || text.contains("|"))
        XCTAssertTrue(text.contains("I fixed two bugs in api.rs:"))
        XCTAssertTrue(text.contains("The login flow"))
        XCTAssertTrue(text.contains("(code omitted)"))
        XCTAssertTrue(text.contains("See a link for details."))
        XCTAssertFalse(text.contains("https://"))
    }

    func testSpeakableTruncatesLongReplies() {
        let para = String(repeating: "This sentence is part of a long answer. ", count: 10)
        let (text, truncated) = VoiceText.speakable([para, para, para, para].joined(separator: "\n\n"), limit: 900)
        XCTAssertTrue(truncated)
        XCTAssertLessThanOrEqual(text.count, 900)
    }

    func testLanguageDetection() {
        XCTAssertEqual(VoiceText.dominantLanguage("Corrigi o bug de login e rodei os testes de novo."), "pt")
        XCTAssertEqual(VoiceText.dominantLanguage("I fixed the login bug and ran the tests again."), "en")
    }

    func testLevelMeter() {
        let format = AVAudioFormat(standardFormatWithSampleRate: 16000, channels: 1)!
        let silent = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 160)!
        silent.frameLength = 160
        XCTAssertEqual(VoiceSession.rms(silent), 0)
        let loud = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 160)!
        loud.frameLength = 160
        for i in 0..<160 { loud.floatChannelData![0][i] = i % 2 == 0 ? 0.9 : -0.9 }
        XCTAssertGreaterThan(VoiceSession.rms(loud), 0.9)
    }
}

final class StartupFailureGuardTests: XCTestCase {
    func testGivesUpAfterRepeatedInstantFailures() {
        var g = StartupFailureGuard()
        XCTAssertFalse(g.record(failed: true, heard: false, lasted: 0.01))
        XCTAssertFalse(g.record(failed: true, heard: false, lasted: 0.01))
        XCTAssertTrue(g.record(failed: true, heard: false, lasted: 0.01), "third instant failure in a row stops the loop")
    }

    func testPausesAndRealSpeechDontCount() {
        var g = StartupFailureGuard()
        // A long silent request ending in "no speech" is a pause, not a broken recognizer.
        for _ in 0..<10 { XCTAssertFalse(g.record(failed: true, heard: false, lasted: 30)) }
        // Normal rollovers that heard something reset the count.
        XCTAssertFalse(g.record(failed: true, heard: false, lasted: 0.1))
        XCTAssertFalse(g.record(failed: true, heard: false, lasted: 0.1))
        XCTAssertFalse(g.record(failed: false, heard: true, lasted: 40))
        XCTAssertFalse(g.record(failed: true, heard: false, lasted: 0.1))
        XCTAssertFalse(g.record(failed: false, heard: false, lasted: 0.1), "finishing without an error never counts")
    }
}

@MainActor
final class VoiceSessionTests: XCTestCase {
    func testFixtureReviewNeverSendsOnItsOwn() async throws {
        let store = LoftStore(api: MockAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("v-\(UUID()).json")))
        let session = VoiceSession(store: store)
        session.applyFixture("review", agentId: "a1")
        XCTAssertEqual(session.phase, .review)
        XCTAssertNotNil(session.cleaned)
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(session.phase, .review, "review waits for an explicit send")
        XCTAssertTrue(store.pending.isEmpty)
        session.discard()
        XCTAssertEqual(session.phase, .idle)
        XCTAssertNil(VoiceSession.savedDraft())
    }

    func testOnlyPermissionFailuresOfferSettings() {
        let store = LoftStore(api: MockAPI(), cache: SnapshotStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("v-\(UUID()).json")))
        let session = VoiceSession(store: store)
        session.applyFixture("failed", agentId: "a1")
        XCTAssertTrue(session.failureNeedsPermission, "mic access off: Loft's settings page fixes it")
        session.applyFixture("failed-startup", agentId: "a1")
        XCTAssertTrue(session.isFailed)
        XCTAssertFalse(session.failureNeedsPermission, "a missing speech model isn't fixed in Loft's settings page")
    }
}

/// Real recognition, end to end: Apple's voice reads ~a minute of text into the
/// chained recognizer with short segments, and the joined result has to keep the
/// words from every segment. Skipped where on-device recognition isn't available.
final class ChainedRecognizerTests: XCTestCase {
    func testSegmentsJoinWithoutLosingWords() async throws {
        // Never prompt: on a fresh simulator `requestAuthorization` shows an alert nobody taps, and the test hangs.
        // Grant it once with `xcrun simctl privacy <udid> grant speech-recognition <bundle id>`.
        guard SFSpeechRecognizer.authorizationStatus() == .authorized else {
            throw XCTSkip("speech recognition isn't authorized on this simulator (grant it with simctl privacy to run this test)")
        }
        guard let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "en-US")), recognizer.isAvailable,
              recognizer.supportsOnDeviceRecognition else {
            throw XCTSkip("on-device speech recognition isn't available here")
        }
        guard AVSpeechSynthesisVoice(language: "en-US") != nil else { throw XCTSkip("no en-US voice to synthesize test speech") }
        let script = [
            "Please look at the settings screen first.", "It crashes when the server address is empty.",
            "Then check the backend endpoint for tasks.", "The cancel button should ask before stopping a run.",
            "After that, run the tests on the simulator.", "Tell me which ones fail and why.",
            "Finally, open a pull request with screenshots.",
        ].joined(separator: " ")
        let buffers = try await synthesize(script)
        guard !buffers.isEmpty else { throw XCTSkip("speech synthesis produced no audio here") }
        let r = await MainActor.run { ChainedSegmentRecognizer(locale: Locale(identifier: "en-US"), segmentDuration: 5, hardLimit: 12) }
        final class Box: @unchecked Sendable { var commits: [String] = []; var error: String? }
        let box = Box()
        await MainActor.run {
            r.onCommit = { box.commits.append($0) }
            r.onError = { box.error = $0 }
        }
        try await r.prepare()
        try await MainActor.run { try r.start() }
        // Feed in real time so segment rollovers happen mid-speech.
        for b in buffers {
            r.append(b)
            try await Task.sleep(for: .seconds(Double(b.frameLength) / b.format.sampleRate))
        }
        try await Task.sleep(for: .seconds(1))
        await r.flush()
        // The recognizer can be reported as supported yet fail to start (e.g. an iOS 26 simulator without the
        // on-device model). That's the environment, not the code: the engine reports it, and so does the test.
        if let error = box.error, box.commits.isEmpty {
            throw XCTSkip("speech recognition couldn't start here: \(error)")
        }
        let commits = box.commits
        let joined = VoiceText.join(commits).lowercased()
        print("chained recognizer: \(commits.count) segments → \(joined)")
        XCTAssertGreaterThan(commits.count, 1, "the audio spanned several segments")
        // Stems, because the simulator's recognizer mishears a synthetic voice now and then
        // ("setting", "back end"). "then" and "finally" start sentences, which is where seams fall.
        for word in ["please", "setting", "crash", "empty", "then", "task", "cancel", "after that", "simulator", "fail", "finally", "screenshot"] {
            XCTAssertTrue(joined.contains(word), "missing “\(word)” in: \(joined)")
        }
    }

    private func synthesize(_ text: String) async throws -> [AVAudioPCMBuffer] {
        let synth = AVSpeechSynthesizer()
        let u = AVSpeechUtterance(string: text)
        u.voice = AVSpeechSynthesisVoice(language: "en-US")
        var out: [AVAudioPCMBuffer] = []
        // `write` signals the end with an empty buffer; a simulator without the voice asset may never send it,
        // so give up after 30 s instead of waiting forever.
        final class Once: @unchecked Sendable {
            let lock = NSLock(); var done = false
            func fire(_ f: () -> Void) { lock.lock(); defer { lock.unlock() }; if !done { done = true; f() } }
        }
        let once = Once()
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            synth.write(u) { buffer in
                guard let pcm = buffer as? AVAudioPCMBuffer else { return }
                if pcm.frameLength == 0 { once.fire { c.resume() } } else { out.append(pcm) }
            }
            DispatchQueue.global().asyncAfter(deadline: .now() + 30) { once.fire { c.resume() } }
        }
        _ = synth
        return out
    }
}

final class VoiceChoiceTests: XCTestCase {
    private func v(_ id: String, _ lang: String, _ q: VoiceChoice.Quality, _ name: String? = nil) -> VoiceChoice {
        VoiceChoice(identifier: id, name: name ?? id, language: lang, quality: q)
    }

    private lazy var installed = [
        v("en-default", "en-US", .standard),
        v("en-premium", "en-US", .premium),
        v("en-gb-enhanced", "en-GB", .enhanced),
        v("pt-pt-premium", "pt-PT", .premium),
        v("pt-br-enhanced", "pt-BR", .enhanced),
        v("pt-br-premium", "pt-BR", .premium),
    ]

    func testAutomaticPrefersQualityThenRegion() {
        XCTAssertEqual(VoiceChoice.pick(language: "en", region: "US", chosenId: nil, voices: installed), "en-premium")
        // Region breaks the tie between two Premium Portuguese voices.
        XCTAssertEqual(VoiceChoice.pick(language: "pt", region: "BR", chosenId: "", voices: installed), "pt-br-premium")
        XCTAssertEqual(VoiceChoice.pick(language: "pt", region: "PT", chosenId: nil, voices: installed), "pt-pt-premium")
        // Quality outranks region.
        XCTAssertEqual(VoiceChoice.pick(language: "en", region: "GB", chosenId: nil, voices: installed), "en-premium")
        XCTAssertNil(VoiceChoice.pick(language: "ja", region: nil, chosenId: nil, voices: installed))
    }

    func testChosenVoiceOnlyForItsLanguage() {
        // A chosen Enhanced voice beats a Premium one when the reply is in its language.
        XCTAssertEqual(VoiceChoice.pick(language: "en", region: "US", chosenId: "en-gb-enhanced", voices: installed), "en-gb-enhanced")
        // A Portuguese reply isn't read by the chosen English voice.
        XCTAssertEqual(VoiceChoice.pick(language: "pt", region: "BR", chosenId: "en-gb-enhanced", voices: installed), "pt-br-premium")
    }

    func testNoveltyVoicesAreNeverAutomatic() {
        let voices = [VoiceChoice(identifier: "bells", name: "Bells", language: "en-US", quality: .premium, isNovelty: true),
                      v("en-default", "en-US", .standard)]
        XCTAssertEqual(VoiceChoice.pick(language: "en", region: "US", chosenId: nil, voices: voices), "en-default")
        // Chosen on purpose, it's still honoured.
        XCTAssertEqual(VoiceChoice.pick(language: "en", region: "US", chosenId: "bells", voices: voices), "bells")
    }

    func testUninstalledChoiceFallsBackToAutomatic() {
        XCTAssertEqual(VoiceChoice.pick(language: "en", region: "US", chosenId: "gone", voices: installed), "en-premium")
    }

    func testPickerOrder() {
        let sections = VoiceChoice.grouped(installed + [v("de-1", "de-DE", .standard)], firstLanguage: "pt")
        XCTAssertEqual(sections.first?.language, "pt", "the user's language comes first")
        XCTAssertEqual(Set(sections.map(\.language)), ["pt", "en", "de"])
        let en = sections.first { $0.language == "en" }!.voices.map(\.identifier)
        XCTAssertEqual(en, ["en-premium", "en-gb-enhanced", "en-default"], "Premium, then Enhanced, then Default")
    }

    func testLanguageCodeAndSamples() {
        XCTAssertEqual(v("x", "pt-BR", .standard).languageCode, "pt")
        XCTAssertEqual(v("x", "zh_CN", .standard).languageCode, "zh")
        XCTAssertTrue(VoiceChoice.sample(for: "pt-BR").hasPrefix("Olá"))
        XCTAssertTrue(VoiceChoice.sample(for: "en-AU").hasPrefix("Hi"))
        XCTAssertFalse(VoiceChoice.sample(for: "ja-JP").isEmpty)
    }
}
