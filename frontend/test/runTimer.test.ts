// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { fmtDone, fmtElapsed, runElapsed } from "../src/lib/runTimer.ts";

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
