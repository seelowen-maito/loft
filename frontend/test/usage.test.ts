// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { costChangeText, resetText, weekResetText } from "../src/lib/usage.ts";

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

test("the home line's reset is the day and 24h time", () => {
  assert.equal(weekResetText(new Date(2026, 9, 29, 4, 0).toISOString(), "en-GB"), "resets Thu 04:00");
  assert.equal(weekResetText(new Date(2026, 9, 29, 16, 30).toISOString(), "en-US"), "resets Thu 16:30");
  assert.equal(weekResetText(null), null);
  assert.equal(weekResetText("nope"), null);
});

test("the cost change against the previous period reads as an arrow and a percentage", () => {
  assert.equal(costChangeText(84.1, 75.09), "▲ 12% vs last");
  assert.equal(costChangeText(46, 50), "▼ 8% vs last");
  assert.equal(costChangeText(0, 10), "▼ 100% vs last");
  assert.equal(costChangeText(30, 10), "▲ 200% vs last");
  assert.equal(costChangeText(10.02, 10), "same as last");
});

test("no percentage when the previous period cost nothing", () => {
  assert.equal(costChangeText(5, 0), null);
  assert.equal(costChangeText(0, 0), null);
});
