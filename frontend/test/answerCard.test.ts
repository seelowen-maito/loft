import { test } from "node:test";
import assert from "node:assert/strict";
import { answerCard, asksSomething, listedOptions, offersAChoice, question, yesNo } from "../src/lib/answerCard.ts";

// The iPhone's cases (ios/LoftTests/ChatRepliesTests.swift), so both apps agree on which messages ask.
test("only a message whose last paragraph, outside code, ends on a question asks", () => {
  assert.ok(asksSomething("Tests are green. Should I merge it now or wait for QA?"));
  assert.ok(asksSomething("Here's what I found.\n\nWant me to open a PR?"));
  assert.ok(asksSomething("Faço o merge agora?"));
  assert.ok(!asksSomething("Why did it fail? A missing key. I added it and the build is green."), "it answered itself");
  assert.ok(!asksSomething("Should I merge?\n\nNever mind, I merged it."));
  assert.ok(!asksSomething("```swift\nlet x = y ?? z\n```"), "a ? in code isn't a question");
  assert.ok(!asksSomething(""));
});

test("a question mark under closing emphasis or quotes still counts; one inside a code block ending the message doesn't", () => {
  assert.ok(asksSomething("**Publish these 2 tickets?**"));
  assert.ok(asksSomething("Should I use “port 7420?”"));
  assert.ok(!asksSomething("Run this?\n\n```sh\nls ?\n```\n\nDone, it ran."));
  assert.ok(asksSomething("Here it is:\n\n```sh\nls\n```\n\nRun it?"));
});

test("an either/or question offers a choice, in English or Portuguese", () => {
  assert.ok(offersAChoice("Should I merge it now or wait for QA?"));
  assert.ok(offersAChoice("Faço o merge agora ou espero?"));
  assert.ok(!offersAChoice("Should I retry the failed run?"));
  assert.ok(!offersAChoice("Should I order the tickets?"), "'or' inside a word");
});

test("Yes/No only for a question answered by yes or no", () => {
  for (const q of ["Publish these 2 tickets?", "Should I retry the failed run?", "Faço o merge agora?", "Tests are green. Want me to open a PR?"]) assert.ok(yesNo(q), q);
  for (const q of ["Which port should it use?", "What do you want to call it?", "Qual porta?", "O que faço agora?", "Merge now or wait for QA?"]) assert.ok(!yesNo(q), q);
});

test("the card quotes the last paragraph, without a list it starts with", () => {
  assert.equal(question("Here's the plan:\n\n- #1 API\n- #2 Web\n\nPublish these 2 tickets?"), "Publish these 2 tickets?");
  assert.equal(question("1. Merge now\n2. Wait for QA\nWhich do you prefer, merging or waiting?"), "Which do you prefer, merging or waiting?");
  assert.equal(question("Two ways.\nShould I merge?"), "Two ways. Should I merge?");
});

test("the numbered or bulleted list right before the question gives the options", () => {
  assert.deepEqual(listedOptions("Two ways:\n\n1. **Merge now**\n2. Wait for `QA`\n\nMerge now or wait for QA?"), ["Merge now", "Wait for QA"]);
  assert.deepEqual(listedOptions("- Rebase,\n  which rewrites history\n- Merge\n- Squash.\nWhich one, rebase or merge or squash?"), ["Rebase", "Merge", "Squash"]);
  assert.deepEqual(listedOptions("1) Ship it or\n\n2) Hold it\n\nShip or hold?"), ["Ship it", "Hold it"], "loose list, trailing 'or' dropped");
  assert.deepEqual(listedOptions("- A\n- B\n\nThat's all.\n\nShip or hold?"), [], "a paragraph between the list and the question");
  assert.deepEqual(listedOptions("Ship or hold?"), []);
});

const m = (id: number, role: string, content: string, kind = "text") => ({ id, role, kind, content });

test("the Lead's ticket list ending 'Publish these 2 tickets?' gets Yes and No, not a button per ticket", () => {
  const c = answerCard([m(1, "user", "plan it"), m(2, "assistant", "Tickets:\n\n- #1 API\n- #2 Web\n\nPublish these 2 tickets?")], false);
  assert.deepEqual(c && { id: c.message.id, question: c.question, answers: c.answers }, { id: 2, question: "Publish these 2 tickets?", answers: ["Yes", "No"] });
});

test("an either/or question gets no Yes/No: its listed options, or only Reply…", () => {
  assert.deepEqual(answerCard([m(1, "assistant", "Merge now or wait for QA?")], false)?.answers, []);
  assert.deepEqual(answerCard([m(1, "assistant", "1. Merge now\n2. Wait for QA\n\nMerge now or wait for QA?")], false)?.answers, ["Merge now", "Wait for QA"]);
  const many = Array.from({ length: 7 }, (_, i) => `- Option ${i}`).join("\n");
  assert.deepEqual(answerCard([m(1, "assistant", `${many}\n\nOption 0 or another?`)], false)?.answers, [], "too many for buttons");
  assert.deepEqual(answerCard([m(1, "assistant", "Which port should it use?")], false)?.answers, [], "an open question: Reply… only");
});

test("no card when the question is in code or followed by more text, after the user's message, or while something else waits on the user", () => {
  assert.equal(answerCard([m(1, "assistant", "```\nok?\n```")], false), null);
  assert.equal(answerCard([m(1, "assistant", "Should I merge?\n\nNever mind, I merged it.")], false), null);
  assert.equal(answerCard([m(1, "assistant", "Should I merge?"), m(2, "user", "Yes")], false), null, "answered");
  assert.equal(answerCard([m(1, "assistant", "Should I merge?"), m(2, "user", "From QA: tests pass")], false), null, "another agent's message came in");
  assert.equal(answerCard([m(1, "assistant", "Should I merge?")], true), null, "a permission, AskUserQuestion or grill");
  assert.equal(answerCard([], false), null);
});

test("tool rows and notices after the question don't hide the card; the agent's next text that doesn't ask does", () => {
  const asked = [m(1, "assistant", "Should I merge?"), m(2, "system", "", "run_end"), m(3, "assistant", "{}", "tool_use")];
  assert.equal(answerCard(asked, false)?.message.id, 1);
  assert.equal(answerCard([...asked, m(4, "assistant", "Merged it anyway.")], false), null);
});
