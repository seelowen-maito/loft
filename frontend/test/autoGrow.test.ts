// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { fitRows } from "../src/lib/autoGrow.ts";

const LINE = 21;

test("a line that wraps onto a second visual line takes two rows", () => {
  assert.deepEqual(fitRows(2 * LINE, LINE, false, 1), { rows: 2, scroll: false });
});

test("short text keeps the minimum rows", () => {
  assert.deepEqual(fitRows(LINE, LINE, false, 2), { rows: 2, scroll: false });
});

test("text past 12 lines stops at 12 and scrolls", () => {
  assert.deepEqual(fitRows(30 * LINE, LINE, false, 1), { rows: 12, scroll: true });
});

test("exactly 12 lines fits without scrolling", () => {
  assert.deepEqual(fitRows(12 * LINE, LINE, false, 1), { rows: 12, scroll: false });
});

test("an empty box takes the minimum, however far its placeholder wraps", () => {
  assert.deepEqual(fitRows(3 * LINE, LINE, true, 1), { rows: 1, scroll: false });
  assert.deepEqual(fitRows(5 * LINE, LINE, true, 2), { rows: 2, scroll: false });
});

test("fractional heights round to the nearest line", () => {
  assert.deepEqual(fitRows(3 * LINE + 2, LINE, false, 1), { rows: 3, scroll: false });
});

test("a grill answer grows to 5 lines, then scrolls, and shrinks back", () => {
  assert.deepEqual(fitRows(3 * LINE, LINE, false, 1, 5), { rows: 3, scroll: false });
  assert.deepEqual(fitRows(5 * LINE, LINE, false, 1, 5), { rows: 5, scroll: false });
  assert.deepEqual(fitRows(9 * LINE, LINE, false, 1, 5), { rows: 5, scroll: true });
  assert.deepEqual(fitRows(LINE, LINE, false, 1, 5), { rows: 1, scroll: false });
});
