// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { tipPosition } from "../src/lib/tipPosition.ts";

const VIEW = { width: 1000, height: 800 };
const TIP = { width: 200, height: 30 };
const mark = (left: number, top: number) => ({ left, top, width: 10, height: 10 });

test("a tooltip sits centered above its mark", () => {
  assert.deepEqual(tipPosition(mark(495, 400), TIP, VIEW), { left: 400, top: 364 });
});

test("near the left edge it stays on screen", () => {
  assert.deepEqual(tipPosition(mark(0, 400), TIP, VIEW), { left: 8, top: 364 });
});

test("near the right edge it stays on screen", () => {
  assert.deepEqual(tipPosition(mark(990, 400), TIP, VIEW), { left: 792, top: 364 });
});

test("with no room above the mark it goes below it", () => {
  assert.deepEqual(tipPosition(mark(495, 20), TIP, VIEW), { left: 400, top: 36 });
});

test("it never leaves the bottom edge", () => {
  assert.deepEqual(tipPosition({ left: 495, top: 10, width: 10, height: 790 }, TIP, VIEW).top, 762);
});

test("a tooltip wider than the screen starts at the left margin", () => {
  assert.equal(tipPosition(mark(495, 400), { width: 1200, height: 30 }, VIEW).left, 8);
});
