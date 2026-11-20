import { test } from "node:test";
import assert from "node:assert/strict";
import { statusOf } from "../src/lib/agentStatus.ts";

const agent = (status: string, o: { running?: boolean; waiting_on?: string[]; every?: number } = {}) =>
  statusOf({ status, running: o.running, waiting_on: o.waiting_on, interval_minutes: o.every ?? 0 }).label;

test("Working comes from the live run, not the saved status", () => {
  assert.equal(agent("running"), "Idle", "saved running with no live run");
  assert.equal(agent("running", { every: 30 }), "Sleeping");
  assert.equal(agent("idle", { running: true }), "Working");
  assert.equal(agent("running", { running: true }), "Working");
});

test("the other states", () => {
  assert.equal(agent("waiting"), "Needs you");
  assert.equal(agent("idle", { running: true, waiting_on: ["480"] }), "Waiting");
  assert.equal(agent("error"), "Error");
  assert.equal(agent("idle"), "Idle");
});
