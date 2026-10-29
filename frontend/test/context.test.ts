// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { CONTEXT_WARN_AT, contextWarns } from "../src/lib/context.ts";

test("the context size warns from 80k tokens", () => {
  assert.equal(CONTEXT_WARN_AT, 80_000);
  assert.equal(contextWarns(0), false);
  assert.equal(contextWarns(79_999), false, "below 80k is neutral");
  assert.equal(contextWarns(80_000), true, "80k itself warns");
  assert.equal(contextWarns(250_000), true);
});
