// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { interviewPath, interviewShows } from "../src/lib/interview.ts";
import type { GrillSession } from "../src/lib/api.ts";

const session = (over: Partial<GrillSession>): GrillSession => ({
  session_id: "s", agent_id: "a", state: "active", end_reason: null, topic: "t", round: 2, total: 3, answered: 1,
  waiting: false, questions: [], started_at: "", updated_at: "", ...over,
});

test("the full-screen interview has its own URL under the agent", () => {
  assert.equal(interviewPath("a1"), "/agents/a1/interview");
});

test("it shows the open round, paused or not, and waits while the agent prepares one", () => {
  assert.equal(interviewShows(session({})), "round");
  assert.equal(interviewShows(session({ state: "paused" })), "round", "a paused round can still be answered");
  assert.equal(interviewShows(session({ waiting: true })), "waiting", "sent: the next round is coming");
  assert.equal(interviewShows(session({ state: "preparing", round: null })), "waiting", "the first round is coming");
});

test("with no interview, or an ended one, it shows none", () => {
  assert.equal(interviewShows(null), "none");
  assert.equal(interviewShows(session({ state: "ended" })), "none");
});
