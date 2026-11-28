import { test } from "node:test";
import assert from "node:assert/strict";
import { NEEDS_YOU_LABEL, taskStatus, toneText } from "../src/lib/taskStatus.ts";
import { urgencyOf } from "../src/lib/needsYou.ts";
import { badgeText, ticketState } from "../src/lib/specViews.ts";

test("every task status has one label and one tone", () => {
  assert.deepEqual(taskStatus("waiting_approval"), { label: "Needs you", tone: "warn" });
  assert.deepEqual(taskStatus("blocked"), { label: "Blocked", tone: "warn" });
  assert.deepEqual(taskStatus("failed"), { label: "Failed", tone: "bad" });
  assert.deepEqual(taskStatus("in_progress"), { label: "In progress", tone: "accent" });
  assert.deepEqual(taskStatus("ready"), { label: "Ready", tone: "info" });
  assert.deepEqual(taskStatus("inbox"), { label: "Inbox", tone: "muted" });
  assert.deepEqual(taskStatus("done"), { label: "Done", tone: "good" });
  assert.deepEqual(taskStatus("cancelled"), { label: "Cancelled", tone: "muted" });
});

test("an unknown or missing status reads as its words, muted", () => {
  assert.deepEqual(taskStatus("on_hold"), { label: "on hold", tone: "muted" });
  assert.deepEqual(taskStatus("waiting_on_ci"), { label: "waiting on ci", tone: "muted" }, "every underscore");
  assert.deepEqual(taskStatus(undefined), { label: "", tone: "muted" });
});

test("each tone has a text colour", () => {
  assert.deepEqual((["good", "accent", "info", "warn", "bad", "muted"] as const).map(toneText), ["text-good", "text-accent", "text-info", "text-warn", "text-bad", "text-fg-3"]);
});

test("waiting on the user reads Needs you for tasks, tickets, Deliveries and Needs you", () => {
  assert.equal(taskStatus("waiting_approval").label, NEEDS_YOU_LABEL);
  assert.equal(ticketState({ state: "needs_human" }).text, NEEDS_YOU_LABEL);
  assert.equal(urgencyOf(2).label, NEEDS_YOU_LABEL);
  assert.equal(badgeText("delivery", { state: "needs_human" } as any), `In Delivery · ${NEEDS_YOU_LABEL}`);
});
