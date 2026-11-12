import assert from "node:assert/strict";
import { test } from "node:test";
import { QUIET_MINUTES, runWatch } from "../src/lib/runWatch.ts";

const now = Date.parse("2026-11-12T12:00:00Z");
const ago = (min: number) => new Date(now - min * 60000).toISOString();
const run = (started: number, output: number) => ({ running: true, last_run_at: ago(started), last_output_at: ago(output), waiting_on: [], waiting_since: null });

test("a quiet run that just started isn't flagged, however old its last output and message", () => {
  assert.equal(runWatch(run(2, QUIET_MINUTES + 30), ago(QUIET_MINUTES + 60), now), null);
});

test("a quiet run started past QUIET_MINUTES is", () => {
  const w = runWatch(run(QUIET_MINUTES + 2, QUIET_MINUTES + 30), ago(QUIET_MINUTES + 60), now);
  assert.equal(w?.kind, "quiet");
  assert.equal(w?.minutes, QUIET_MINUTES + 2);
  assert.equal(w?.text, `No activity for ${QUIET_MINUTES + 2} min`);
});

test("recent output or a recent message keeps it from being flagged", () => {
  assert.equal(runWatch(run(QUIET_MINUTES + 5, 1), ago(QUIET_MINUTES + 60), now), null);
  assert.equal(runWatch(run(QUIET_MINUTES + 5, QUIET_MINUTES + 5), ago(1), now), null);
});

test("an agent that isn't running is never flagged", () => {
  assert.equal(runWatch({ ...run(60, 60), running: false }, undefined, now), null);
});

test("waiting on background work reads as waiting, from when it started waiting", () => {
  const w = runWatch({ ...run(30, 30), waiting_on: ["Run the QA suite", "Watch the logs"], waiting_since: ago(4) }, undefined, now);
  assert.deepEqual(w, { kind: "waiting", minutes: 4, text: "Waiting on a background task: Run the QA suite (+1 more)" });
});
