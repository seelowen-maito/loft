// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { startDeliveryLabel } from "../src/lib/startDelivery.ts";

test("idle, the button offers to start; pending, it says it's starting", () => {
  assert.equal(startDeliveryLabel(false), "Start delivery");
  assert.equal(startDeliveryLabel(true), "Starting delivery…");
});
