// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { fmtDone, fmtElapsed, lastFinishedRun, runElapsed } from "../src/lib/runTimer.ts";

test("the timer shows seconds, then minutes, then hours", () => {
  assert.equal(fmtElapsed(0), "0s");
  assert.equal(fmtElapsed(45_900), "45s");
  assert.equal(fmtElapsed(60_000), "1m 0s");
  assert.equal(fmtElapsed(143_000), "2m 23s");
  assert.equal(fmtElapsed(3_849_000), "1h 4m 9s");
});

test("a start in the future (clock skew) reads as 0s", () => {
  assert.equal(fmtElapsed(-5_000), "0s");
});

test("elapsed time counts from the server's run start", () => {
  const start = "2026-10-29T16:20:00.000Z";
  assert.equal(runElapsed(start, Date.parse("2026-10-29T16:22:23.400Z")), 143_400);
  assert.equal(fmtElapsed(runElapsed(start, Date.parse("2026-10-29T16:22:23.400Z"))), "2m 23s");
  assert.equal(runElapsed(null, Date.now()), 0);
  assert.equal(runElapsed("not a date", Date.now()), 0);
});

test("a finished run reads Done in <duration> · <day> <time>", () => {
  const end = new Date(2026, 9, 29, 16, 23); // a Thursday, local time
  assert.equal(fmtDone(143_000, end, "en-GB"), "Done in 2m 23s · Thu 16:23");
  assert.equal(fmtDone(143_000, end.toISOString(), "en-GB"), "Done in 2m 23s · Thu 16:23");
});

test("the day and time follow the locale", () => {
  const end = new Date(2026, 9, 29, 16, 23);
  const day = end.toLocaleDateString("de-DE", { weekday: "short" });
  const time = end.toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });
  assert.equal(fmtDone(5_000, end, "de-DE"), `Done in 5s · ${day} ${time}`);
  assert.notEqual(day, "Thu");
});

const msg = (kind: string, run_id: string | null, at: string, meta: Record<string, any> = {}) => ({ kind, run_id, created_at: `2026-10-29T16:${at}Z`, meta }) as any;

test("a finished run is measured from its run_start to its run_end, like the live timer", () => {
  const run = [msg("text", null, "20:00"), msg("run_start", "r1", "20:00"), msg("text", "r1", "21:00"), msg("run_end", "r1", "22:23", { duration_ms: 141_000 })];
  assert.deepEqual(lastFinishedRun(run), { durationMs: 143_000, endedAt: "2026-10-29T16:22:23Z" });
});

test("a stopped run has no duration of its own: it counts from run_start to run_end", () => {
  const run = [msg("run_start", "r1", "20:00"), msg("run_end", "r1", "20:30", { stopped: true })];
  assert.deepEqual(lastFinishedRun(run), { durationMs: 30_000, endedAt: "2026-10-29T16:20:30Z" });
});

test("the run_end's duration_ms is used when its run_start is not loaded", () => {
  assert.deepEqual(lastFinishedRun([msg("run_end", "r1", "22:23", { duration_ms: 5_000 })]), { durationMs: 5_000, endedAt: "2026-10-29T16:22:23Z" });
});

test("a /clear after the run hides the line", () => {
  const run = [msg("run_start", "r1", "20:00"), msg("run_end", "r1", "20:30", { duration_ms: 30_000 }), msg("clear", null, "21:00")];
  assert.equal(lastFinishedRun(run), null);
});

test("a run that failed without a run_end shows no line, not the previous run's", () => {
  const run = [
    msg("run_start", "r1", "20:00"), msg("run_end", "r1", "20:13", { duration_ms: 13_000 }),
    msg("text", null, "21:00"), msg("run_start", "r2", "21:00"), msg("error", "r2", "21:01"),
  ];
  assert.equal(lastFinishedRun(run), null);
});

test("no runs yet, no line", () => {
  assert.equal(lastFinishedRun([]), null);
  assert.equal(lastFinishedRun([msg("text", null, "20:00")]), null);
});
