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

test("a paused Delivery says so", () => {
  assert.deepEqual(at("paused"), { tone: "paused", text: "Paused: Loft sends no wakes until you resume it." });
});

test("a ticket or the final review waiting for the user is Needs you, over running", () => {
  assert.equal(at("running", { needs_human: true })?.tone, "needs_you");
  assert.equal(at("needs_human", { needs_human: true })?.text, "Needs you: too many rounds of changes in the final review.");
  assert.equal(at("stalled", { stall_reason: "no_call", needs_human: true })?.tone, "stalled", "a stalled one says so");
});

test("the server's state line shows as is", () => {
  const line = (state_line: { tone: string; text: string } | null) => deliveryStateLine({ state: "stalled", started: true, stall_reason: "no_call", state_line });
  assert.deepEqual(line({ tone: "stalled", text: "Stalled: reworded on the server." }), { tone: "stalled", text: "Stalled: reworded on the server." });
  assert.deepEqual(line({ tone: "needs_you", text: "Needs you." }), { tone: "needs_you", text: "Needs you." });
  assert.deepEqual(line({ tone: "blocked", text: "Blocked: a new state." }), { tone: "running", text: "Blocked: a new state." }, "an unknown tone takes running's colour");
  assert.equal(line(null), null);
  assert.equal(deliveryStateLine({ state: "planned", started: false, state_line: { tone: "running", text: "x" } }), null, "not before it started");
});

test("an older server without a state line gets today's wording", () => {
  assert.equal(at("stalled", { stall_reason: "restart" })?.text, "Stalled: its run was lost when Loft restarted.");
  assert.deepEqual(deliveryStateLine({ state: "running", started: true }), { tone: "running", text: "Running." });
});

test("Resume shows only on a stalled or paused Delivery, or a ticket or final review waiting for the user", () => {
  const card = (state: string, can_resume: boolean, started = true) => deliveryButtons({ started, can_resume }).resume;
  assert.equal(card("stalled", true), true);
  assert.equal(card("paused", true), true);
  assert.equal(card("needs_human", true), true, "the final review waits for the user");
  assert.equal(card("running", true), true, "a ticket waits for the user");
  for (const s of ["running", "final_review", "final_fixes", "ready", "settled", "merged"]) assert.equal(card(s, false), false, s);
  assert.equal(card("planned", false, false), false);
  assert.deepEqual(deliveryButtons(null), { pause: false, resume: false, reopen: false });
});

test("Reopen delivery shows only when the server says a done Delivery has new tickets", () => {
  assert.equal(deliveryButtons({ started: true, can_reopen: true }).reopen, true);
  assert.equal(deliveryButtons({ started: true, can_reopen: false }).reopen, false);
  assert.equal(deliveryButtons({ started: true }).reopen, false, "an older server says nothing");
  assert.equal(deliveryButtons({ started: false, can_reopen: true }).reopen, false);
});

test("an older server sends no flag: no button", () => {
  assert.deepEqual(deliveryButtons({ started: true }), { pause: false, resume: false, reopen: false });
});

test("Pause shows only while the Delivery runs, Resume while it's paused", () => {
  const card = (can_pause: boolean, can_resume: boolean, started = true) => deliveryButtons({ started, can_pause, can_resume });
  // As the server says for each state: running, final review and fixes pause; a paused one resumes.
  for (const s of ["running", "final_review", "final_fixes"]) assert.deepEqual(card(true, false), { pause: true, resume: false, reopen: false }, s);
  assert.deepEqual(card(false, true), { pause: false, resume: true, reopen: false }, "paused");
  assert.deepEqual(card(true, false, false), { pause: false, resume: false, reopen: false }, "not started");
});
