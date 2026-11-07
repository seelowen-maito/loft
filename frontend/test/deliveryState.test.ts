import { test } from "node:test";
import assert from "node:assert/strict";
import { deliveryButtons, deliveryStateLine } from "../src/lib/deliveryState.ts";

const at = (state: string, more: { stall_reason?: string | null; needs_human?: boolean; started?: boolean } = {}) =>
  deliveryStateLine({ state, started: more.started ?? true, stall_reason: more.stall_reason ?? null, needs_human: more.needs_human ?? false });

test("no line before the Delivery started", () => {
  assert.equal(at("planned", { started: false }), null);
  assert.equal(deliveryStateLine(null), null);
});

test("running, final review and done", () => {
  assert.deepEqual(at("running"), { tone: "running", text: "Running." });
  assert.equal(at("final_review")?.tone, "final");
  assert.equal(at("final_fixes")?.text, "Final review: the Dev is fixing what the Code reviewer found.");
  for (const s of ["ready", "settled", "merged"]) assert.equal(at(s)?.tone, "done", s);
});

test("a stalled Delivery says why", () => {
  assert.deepEqual(at("stalled", { stall_reason: "run_failed" }), { tone: "stalled", text: "Stalled: the run Loft woke for its step failed." });
  assert.equal(at("stalled", { stall_reason: "no_call" })?.text, "Stalled: the run Loft woke for its step ended without its call.");
  assert.equal(at("stalled", { stall_reason: "restart" })?.text, "Stalled: its run was lost when Loft restarted.");
});

test("a ticket or the final review waiting for the user is Needs you, over running", () => {
  assert.equal(at("running", { needs_human: true })?.tone, "needs_you");
  assert.equal(at("needs_human", { needs_human: true })?.text, "Needs you: too many rounds of changes in the final review.");
  assert.equal(at("stalled", { stall_reason: "no_call", needs_human: true })?.tone, "stalled", "a stalled one says so");
});

test("Resume shows only on a stalled Delivery, or a ticket or final review waiting for the user", () => {
  const card = (state: string, can_resume: boolean, started = true) => deliveryButtons({ started, can_resume });
  assert.deepEqual(card("stalled", true), { resume: true });
  assert.deepEqual(card("needs_human", true), { resume: true }, "the final review waits for the user");
  assert.deepEqual(card("running", true), { resume: true }, "a ticket waits for the user");
  for (const s of ["running", "final_review", "final_fixes", "ready", "settled", "merged"]) assert.deepEqual(card(s, false), { resume: false }, s);
  assert.deepEqual(card("planned", false, false), { resume: false });
  assert.deepEqual(deliveryButtons(null), { resume: false });
  assert.deepEqual(deliveryButtons({ started: true }), { resume: false }, "an older server sends no can_resume");
});
