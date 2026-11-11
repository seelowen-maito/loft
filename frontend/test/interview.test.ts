// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { interviewPath, interviewShows, nextQuestion, questionAnchor } from "../src/lib/interview.ts";
import type { GrillQuestion, GrillSession } from "../src/lib/api.ts";

const session = (over: Partial<GrillSession>): GrillSession => ({
  session_id: "s", agent_id: "a", state: "active", end_reason: null, topic: "t", round: 2, total: 3, answered: 1,
  waiting: false, questions: [{ id: "q", state: "open" } as GrillQuestion], started_at: "", updated_at: "", ...over,
});

test("the full-screen interview has its own URL under the agent", () => {
  assert.equal(interviewPath("a1"), "/agents/a1/interview");
});

test("it shows the open round, paused or not, and waits while the agent prepares one", () => {
  assert.equal(interviewShows(session({})), "round");
  assert.equal(interviewShows(session({ state: "paused" })), "round", "a paused round can still be answered");
  assert.equal(interviewShows(session({ waiting: true })), "waiting", "sent: the next round is coming");
  assert.equal(interviewShows(session({ state: "preparing", round: null })), "waiting", "the first round is coming");
  assert.equal(interviewShows(session({ questions: null })), "waiting", "a round whose questions haven't loaded isn't a blank page");
});

test("with no interview, or an ended one, it shows none", () => {
  assert.equal(interviewShows(null), "none");
  assert.equal(interviewShows(session({ state: "ended" })), "none");
});

const q = (id: string, state: GrillQuestion["state"]) => ({ id, state }) as GrillQuestion;

test("the bar points at the round's first unanswered question", () => {
  const qs = [q("a", "answered"), q("b", "discuss"), q("c", "open"), q("d", "open")];
  assert.deepEqual(nextQuestion(session({ questions: qs })), { n: 3, total: 4, id: "c" });
  assert.deepEqual(nextQuestion(session({ questions: [q("a", "decided"), q("b", "open")], state: "paused" })), { n: 2, total: 2, id: "b" });
});

test("with every question answered, or no open round, there's nowhere to jump", () => {
  assert.equal(nextQuestion(session({ questions: [q("a", "answered"), q("b", "decided")] })), null);
  assert.equal(nextQuestion(session({ questions: [q("a", "open")], waiting: true })), null);
  assert.equal(nextQuestion(null), null);
});

test("a question card's anchor names its session, round and question", () => {
  assert.equal(questionAnchor("s1", 2, "q3"), "s1:2:q3");
});
