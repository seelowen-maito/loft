// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { costChange, exactDollars, resetText, wholeDollars } from "../src/lib/usage.ts";

const NOW = new Date(2026, 9, 27, 12, 0).getTime(); // Tue 12:00 local

test("a reset reads as relative, then day and 24h time", () => {
  assert.equal(resetText(new Date(2026, 9, 27, 14, 0).toISOString(), NOW, "en-GB"), "Resets in 2h · Tue 14:00");
  assert.equal(resetText(new Date(2026, 9, 27, 12, 25).toISOString(), NOW, "en-GB"), "Resets in 25m · Tue 12:25");
  assert.equal(resetText(new Date(2026, 9, 30, 21, 5).toISOString(), NOW, "en-GB"), "Resets in 3d · Fri 21:05");
});

test("a reset already due reads as in a moment", () => {
  assert.equal(resetText(new Date(2026, 9, 27, 11, 59).toISOString(), NOW, "en-GB"), "Resets in a moment · Tue 11:59");
});

test("the time stays 24h even in a 12h locale", () => {
  assert.equal(resetText(new Date(2026, 9, 27, 14, 0).toISOString(), NOW, "en-US"), "Resets in 2h · Tue 14:00");
});

test("no or bad reset time says unknown", () => {
  assert.equal(resetText(null, NOW), "Reset time unknown");
  assert.equal(resetText(undefined, NOW), "Reset time unknown");
  assert.equal(resetText("nope", NOW), "Reset time unknown");
});

test("the home line's cost is whole dollars, its tooltip the exact amount", () => {
  assert.equal(wholeDollars(183.14), "$183");
  assert.equal(wholeDollars(182.5), "$183");
  assert.equal(wholeDollars(1438.75), "$1,439");
  assert.equal(wholeDollars(0), "$0");
  assert.equal(exactDollars(183.14), "$183.14");
  assert.equal(exactDollars(1438.7), "$1,438.70");
});

test("the cost change is a short tag: down is good, up is warn, same is muted", () => {
  assert.deepEqual(costChange(84.1, 75.09), { text: "▲12%", tone: "warn" });
  assert.deepEqual(costChange(46, 50), { text: "▼8%", tone: "good" });
  assert.deepEqual(costChange(183, 832), { text: "▼78%", tone: "good" });
  assert.deepEqual(costChange(0, 10), { text: "▼100%", tone: "good" });
  assert.deepEqual(costChange(30, 10), { text: "▲200%", tone: "warn" });
  assert.deepEqual(costChange(10.02, 10), { text: "same", tone: "muted" });
});

test("no change tag when the previous period cost nothing", () => {
  assert.equal(costChange(5, 0), null);
  assert.equal(costChange(0, 0), null);
});
