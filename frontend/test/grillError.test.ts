// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { grillError } from "../src/lib/api.ts";

// The server's 409 codes, with the same text as the app's `LoftStore.grillFailure`.
test("an ended interview says so", () => {
  assert.equal(grillError(new Error("grill_closed")), "This interview has already ended.");
});

test("a sent round or a closed question both read as a sent round", () => {
  assert.equal(grillError(new Error("round_closed")), "This round was already sent.");
  assert.equal(grillError(new Error("question_closed")), "This round was already sent.");
});

test("any other message passes through unchanged", () => {
  assert.equal(grillError(new Error("500 Internal Server Error")), "500 Internal Server Error");
});
