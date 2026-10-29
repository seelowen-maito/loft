// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { commandSpan } from "../src/lib/commandHighlight.ts";

const NAMES = ["/clear", "/compact", "/grill", "/code"];

test("a command typed in full is coloured", () => {
  assert.equal(commandSpan("/grill", NAMES), 6);
  assert.equal(commandSpan("/GRILL", NAMES), 6, "commands run in any case");
});

test("a partial command isn't, even when it names another one's start", () => {
  assert.equal(commandSpan("/gri", NAMES), 0);
  assert.equal(commandSpan("/co", NAMES), 0);
  assert.equal(commandSpan("/", NAMES), 0);
});

test("an unknown command isn't", () => {
  assert.equal(commandSpan("/foo", NAMES), 0);
  assert.equal(commandSpan("/grills", NAMES), 0);
});

test("arguments after a command keep only the command coloured", () => {
  assert.equal(commandSpan("/grill some topic", NAMES), 6);
  assert.equal(commandSpan("/compact\nkeep the plan", NAMES), 8);
});

test("text not starting with a slash is never coloured", () => {
  assert.equal(commandSpan("", NAMES), 0);
  assert.equal(commandSpan(" /grill", NAMES), 0);
  assert.equal(commandSpan("try /grill", NAMES), 0);
});
