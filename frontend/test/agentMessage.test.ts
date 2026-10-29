// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { fyiTitle, isFyi } from "../src/lib/agentMessage.ts";

const lead = { id: "l", name: "Lead" };

test("an FYI from another agent gets the mark", () => {
  assert.equal(isFyi({ role: "user", meta: { from_agent: lead, fyi: true } }), true);
});

test("needs-action and user messages look as before", () => {
  assert.equal(isFyi({ role: "user", meta: { from_agent: lead } }), false, "needs action");
  assert.equal(isFyi({ role: "user", meta: { fyi: true } }), false, "not from an agent");
  assert.equal(isFyi({ role: "user", meta: null }), false, "the user's own message");
  assert.equal(isFyi({ role: "assistant", meta: { from_agent: lead, fyi: true } }), false);
});

test("the mark explains the agent reads it on its next run", () => {
  assert.equal(fyiTitle("Dev"), "FYI: delivered without waking Dev. It reads this at the start of its next run.");
});
