import XCTest
@testable import Loft

final class VoiceSendTests: XCTestCase {
    private func read(_ s: String) -> VoiceSend.Reading { VoiceSend.read(s) }

    func testCommandsOnTheirOwn() {
        XCTAssertEqual(read("Send.").command, .send)
        XCTAssertEqual(read("ok, send it please").command, .send)
        XCTAssertEqual(read("Send to orchestrator").command, .send)
        XCTAssertEqual(read("Manda pro orquestrador").command, .send)
        XCTAssertEqual(read("Envia").command, .send)
        XCTAssertEqual(read("Review").command, .review)
        XCTAssertEqual(read("Review before sending.").command, .review)
        XCTAssertEqual(read("Deixa eu revisar").command, .review)
        XCTAssertEqual(read("send").text, "", "a command alone carries no text")
    }

    func testCommandsAtTheEndAreStripped() {
        let r = read("Fix the login bug on the settings screen, send to orchestrator")
        XCTAssertEqual(r.command, .send)
        XCTAssertEqual(r.text, "Fix the login bug on the settings screen")
        let pt = read("Corrige o bug do login, manda pro orquestrador por favor")
        XCTAssertEqual(pt.command, .send)
        XCTAssertEqual(pt.text, "Corrige o bug do login")
        let now = read("Deploy the docs site, send it now")
        XCTAssertEqual(now.command, .send, "\"now\" is part of the phrase here, not a filler")
        XCTAssertEqual(now.text, "Deploy the docs site")
        let rv = read("Write the release notes for version two. Let me review")
        XCTAssertEqual(rv.command, .review)
        XCTAssertEqual(rv.text, "Write the release notes for version two.")
    }

    func testOrdinarySentencesAreNotCommands() {
        for s in ["Please send the invoice to the client", "Review the pull request and merge it", "I want you to review",
                  "Can you send it", "Envia o relatório pro cliente", "Revisa o código do backend"] {
            let r = read(s)
            XCTAssertNil(r.command, s)
            XCTAssertEqual(r.text, s, s)
            XCTAssertFalse(r.saidToFinish, s)
        }
    }

    func testEndPhrasesFinishWithoutACommand() {
        let r = read("Run the tests again, that's it")
        XCTAssertNil(r.command)
        XCTAssertEqual(r.text, "Run the tests again")
        XCTAssertTrue(r.saidToFinish)
        XCTAssertEqual(read("Roda os testes, pronto").text, "Roda os testes")
    }

    func testNewCommands() {
        XCTAssertEqual(read("Submit").command, .send)
        XCTAssertEqual(read("go ahead").command, .send)
        XCTAssertEqual(read("Send it.").command, .send)
        XCTAssertEqual(read("Cancel").command, .discard)
        XCTAssertEqual(read("Discard that").command, .discard)
        XCTAssertEqual(read("Esquece").command, .discard)
        XCTAssertEqual(read("Start over").command, .startOver)
        XCTAssertEqual(read("Começa de novo").command, .startOver)
        XCTAssertEqual(read("let me check").command, .review)
        // Inside a sentence they're just words.
        for s in ["Submit the form on the settings page", "Go ahead with the migration plan", "Cancel the subscription for that user",
                  "Discard the old branch", "Start over the deploy from scratch"] {
            XCTAssertNil(read(s).command, s)
        }
    }

    func testACommandSaidAfterAPauseCountsOnItsOwn() {
        // "Deploy the docs site … [pause] … send": the last utterance is just "send".
        let base = "Deploy the docs site"
        let r = VoiceSend.command(in: base + " send", sincePause: base)
        XCTAssertEqual(r.command, .send)
        XCTAssertEqual(r.text, base, "the command isn't part of the message")
        XCTAssertEqual(VoiceSend.command(in: base + ". Review.", sincePause: base).command, .review)
        XCTAssertEqual(VoiceSend.command(in: base + " cancel", sincePause: base).command, .discard)
        // Without a pause before it, "…and send" is part of the sentence.
        XCTAssertNil(VoiceSend.command(in: "Write the report and send", sincePause: "").command)
        // More words after the pause: the message goes on.
        XCTAssertNil(VoiceSend.command(in: base + " and send the link to QA", sincePause: base).command)
        // A distinctive phrase at the end still works without a pause.
        XCTAssertEqual(VoiceSend.command(in: base + ", send to orchestrator", sincePause: "").command, .send)
    }

    func testAnswersToTheQuestion() {
        for s in ["Yes", "yeah", "Sim", "yes please", "send", "Manda", "go ahead", "ok, sure"] {
            XCTAssertEqual(VoiceSend.answer(s), .yes, s)
        }
        for s in ["No", "não", "not yet", "review", "wait", "deixa eu revisar", "Espera"] {
            XCTAssertEqual(VoiceSend.answer(s), .no, s)
        }
        XCTAssertEqual(VoiceSend.answer("cancel"), .discard)
        // Anything else is more of the message, never a yes.
        for s in ["and also the blog", "yes and add the tests too", "no idea about the backend", "", "the docs"] {
            XCTAssertNil(VoiceSend.answer(s), s)
        }
    }

    func testTheQuestionIsAskedOncePerQuietSpell() {
        let t0 = Date()
        var p = VoiceSend.Prompt(startedAt: t0)
        XCTAssertFalse(p.shouldAsk(hasText: false, now: t0.addingTimeInterval(30)), "nothing said: nothing to submit")
        XCTAssertFalse(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(4.9)), "a 4.9 s pause is still thinking")
        XCTAssertTrue(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(5)))
        p.didAsk()
        XCTAssertFalse(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(60)), "asked once; not again in the same quiet")
        p.heardWords(at: t0.addingTimeInterval(61))
        XCTAssertFalse(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(64)))
        XCTAssertTrue(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(66)), "new words, new quiet spell: ask again")
    }

    func testTheQuestionSpeaksTheDictationsLanguage() {
        XCTAssertEqual(VoiceSend.question(for: "Deploy the docs site and tell QA when it's live"), "Would you like to submit this?")
        XCTAssertEqual(VoiceSend.question(for: "Faz o deploy do site de documentação e avisa o QA"), "Quer enviar isto?")
    }

    func testStopOnItsOwnEndsWithoutSending() {
        for said in ["Stop.", "ok, stop", "Stop listening", "Para", "parar", "Chega!", "tchau"] {
            let r = read(said)
            XCTAssertEqual(r.command, .stop, said)
            XCTAssertTrue(r.saidToFinish, said)
        }
        // Inside a message it's just a word.
        XCTAssertNil(read("Stop the deploy on staging").command)
        XCTAssertNil(read("Tell the tester to stop").command)
        XCTAssertNil(read("Para o deploy").command)
    }

    func testWordsAddedSinceAPause() {
        let base = "Deploy the docs site"
        XCTAssertNil(VoiceSend.added(since: base, now: "Deploy the docs site."), "punctuation isn't speech")
        XCTAssertNil(VoiceSend.added(since: base, now: "deploy the Docs sight"), "a rewritten word isn't speech")
        XCTAssertEqual(VoiceSend.added(since: base, now: "Deploy the docs site. Review"), "Review")
        XCTAssertEqual(VoiceSend.added(since: base, now: "Deploy the docs site — and the blog too"), "and the blog too")
        XCTAssertEqual(VoiceSend.read(VoiceSend.added(since: base, now: base + " send it")!).command, .send)
        XCTAssertEqual(VoiceSend.read(VoiceSend.added(since: base, now: base + ", review.")!).command, .review)
        XCTAssertNil(VoiceSend.read(VoiceSend.added(since: base, now: base + " and the blog")!).command, "more of the message")
    }
}

final class OpenEndingTests: XCTestCase {
    /// 24 finished and 24 unfinished dictations, English and Portuguese (the set the on-device model was measured
    /// on). The rule never marks a finished one, and catches at least half of the unfinished.
    private let finished = [
        "Deploy the docs site", "Can you check why the build failed on main?", "Run the tests again and tell me when they pass",
        "Merge the settings PR once the checks are green", "Fix the typo in the README", "What is everyone working on right now",
        "Open a PR with the login fix and ask QA to test it", "Restart the backend agent", "Please review the security findings from yesterday",
        "Add dark mode to the settings screen", "Why is the iOS build taking so long", "Stop the docs agent", "Roda os testes de novo",
        "Faz o deploy do site de documentação", "Por que o build falhou", "Abre um PR com a correção do login",
        "Pede pro QA testar o fluxo de pagamento", "Atualiza o README com os passos novos", "O que cada agente está fazendo agora",
        "Para o agente de docs", "Corrige o erro de digitação no título", "Revisa os achados de segurança de ontem", "Check the logs", "Ship it",
        "what should I do", "I think so", "turn it on", "the answer is no",
    ]
    private let unfinished = [
        "The problem is that when I open the settings", "First fix the login, second", "And another thing",
        "I want you to look at the payments flow because", "Can you change the color of the", "So what I'm thinking is that maybe we should",
        "Make the button bigger and also", "There are three things. One, the build is slow. Two", "Ok so the idea is",
        "Tell the backend agent that the endpoint for", "When the user taps on the notification it should", "I noticed that the chat sometimes",
        "O problema é que quando eu abro", "Primeiro corrige o login, depois", "E mais uma coisa", "Eu quero que você olhe o fluxo de pagamento porque",
        "Muda a cor do", "Então a ideia é", "Fala pro agente de backend que o endpoint de", "Quando o usuário toca na notificação ela deveria",
        "Percebi que o chat às vezes", "São três coisas. Um, o build está lento. Dois", "Let me think, the thing I want is", "A questão é",
    ]

    func testAFinishedDictationIsNeverHeldBack() {
        for s in finished { XCTAssertFalse(VoiceSend.endsOpen(s), s) }
    }

    func testMostSentencesLeftOpenAreCaught() {
        let caught = unfinished.filter(VoiceSend.endsOpen)
        XCTAssertGreaterThanOrEqual(caught.count, unfinished.count / 2, "caught: \(caught)")
        for s in ["Fix the login bug and", "the list:", "first the backend,", "Corrige o bug e", "porque"] { XCTAssertTrue(VoiceSend.endsOpen(s), s) }
    }

    func testAnOpenEndingOnlyMovesWhenToAsk() {
        let t0 = Date()
        var p = VoiceSend.Prompt(startedAt: t0)
        p.leftOpen()
        XCTAssertFalse(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(7.9)))
        XCTAssertTrue(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(8)))
        p.heardWords(at: t0.addingTimeInterval(9))
        XCTAssertTrue(p.shouldAsk(hasText: true, now: t0.addingTimeInterval(14)), "new words: back to 5 s")
    }
}
