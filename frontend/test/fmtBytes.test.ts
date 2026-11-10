// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { fmtBytes } from "../src/lib/format.ts";

test("sizes read like Finder's", () => {
  assert.equal(fmtBytes(0), "0 bytes");
  assert.equal(fmtBytes(980), "980 bytes");
  assert.equal(fmtBytes(4_200_000), "4.2 MB");
  assert.equal(fmtBytes(12_400_000_000), "12 GB");
});
