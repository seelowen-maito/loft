// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { commandSpan, highlightParts, mentionRanges } from "../src/lib/commandHighlight.ts";

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

const AGENTS = ["Loft Dev", "Loft Reviewer", "QA", "Web Developer", "Web Dev"];
const coloured = (text: string, commands = NAMES) =>
  highlightParts(text, commands, AGENTS).filter((p) => p.accent).map((p) => p.text);

test("a mention of a known agent is coloured, in any case, wherever it is", () => {
  assert.deepEqual(mentionRanges("@Loft Dev can you look?", AGENTS), [[0, 9]]);
  assert.deepEqual(mentionRanges("ask @loft reviewer and @QA, please", AGENTS), [[4, 18], [23, 26]]);
  assert.deepEqual(mentionRanges("ping (@QA)", AGENTS), [[6, 9]]);
});

test("only a whole name is a mention", () => {
  assert.deepEqual(mentionRanges("@Lof", AGENTS), []);
  assert.deepEqual(mentionRanges("@Loft Devx", AGENTS), []);
  assert.deepEqual(mentionRanges("@Loft Dev-ops", AGENTS), []);
  assert.deepEqual(mentionRanges("@Web Developer here", AGENTS), [[0, 14]], "longest name wins");
  assert.deepEqual(mentionRanges("@Loft Dev hi", ["Loft", "Loft Dev"]), [[0, 9]], "longest name wins, whatever the order");
});

test("an email, a file path or an unknown agent isn't a mention", () => {
  assert.deepEqual(mentionRanges("mail sam@QA.com", AGENTS), []);
  assert.deepEqual(mentionRanges("sam@QA hi", AGENTS), [], "only an @ that starts a word");
  assert.deepEqual(mentionRanges("read @qa/report.md and @QA.txt", AGENTS), []);
  assert.deepEqual(mentionRanges("ask @QA. Thanks", AGENTS), [[4, 7]], "a closing period still mentions");
  assert.deepEqual(mentionRanges("@Nobody hi", AGENTS), []);
  assert.deepEqual(mentionRanges("@QA hi", []), []);
});

test("a command and mentions colour together, and the parts rebuild the text", () => {
  const text = "/grill @Loft Dev and @qa on this";
  assert.deepEqual(coloured(text), ["/grill", "@Loft Dev", "@qa"]);
  assert.equal(highlightParts(text, NAMES, AGENTS).map((p) => p.text).join(""), text);
  assert.deepEqual(coloured("/nope @QA"), ["@QA"]);
  assert.deepEqual(highlightParts("plain text", NAMES, AGENTS), [{ text: "plain text", accent: false }]);
  assert.deepEqual(highlightParts("", NAMES, AGENTS), []);
});
