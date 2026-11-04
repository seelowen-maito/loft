// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { commandAt, completeCommand, highlightParts, mentionRanges, messageCommand } from "../src/lib/commandHighlight.ts";

const NAMES = ["/clear", "/compact", "/grill", "/code", "/resume"];
const span = (text: string) => {
  const c = messageCommand(text, NAMES);
  return c ? text.slice(c.start, c.end) : null;
};

test("a command typed in full is the message's command", () => {
  assert.equal(span("/grill"), "/grill");
  assert.equal(span("/GRILL"), "/GRILL", "commands run in any case");
});

test("a partial or unknown command isn't", () => {
  assert.equal(span("/gri"), null);
  assert.equal(span("/co"), null);
  assert.equal(span("/"), null);
  assert.equal(span("/foo"), null);
  assert.equal(span("/grills"), null);
});

test("a command is found wherever it starts a word, arguments or not", () => {
  assert.equal(span("/grill some topic"), "/grill");
  assert.equal(span("/compact\nkeep the plan"), "/compact");
  assert.equal(span(" /grill"), "/grill");
  assert.equal(span("try /grill"), "/grill");
  assert.equal(span("keep notes\n/compact"), "/compact");
});

test("a slash inside a word is no command", () => {
  assert.equal(span("src/grill"), null);
  assert.equal(span(""), null);
});

test("only the first command counts: a later one is plain text", () => {
  const text = "/compact then /grill";
  assert.deepEqual(messageCommand(text, NAMES), { start: 0, end: 8, word: "/compact" });
});

/** The /word the menu opens on with the caret at the `|` of `marked`, or null. */
const typed = (marked: string) => {
  const caret = marked.indexOf("|");
  return commandAt(marked.replace("|", ""), caret, NAMES)?.word ?? null;
};

test("the menu opens on a / at the start of the message, as before", () => {
  assert.equal(typed("/|"), "/");
  assert.equal(typed("/gr|"), "/gr");
  assert.equal(typed("/grill|"), "/grill");
  assert.equal(typed("/g|r"), "/gr", "the caret may sit anywhere after the /");
  assert.equal(typed("|/gr"), null, "not before the /");
});

test("the menu opens on a / at the start of any word mid-message", () => {
  assert.equal(typed("fix the login bug /gr|"), "/gr");
  assert.equal(typed("fix the login bug\n/co|"), "/co");
  assert.equal(typed("start /gr| and more after"), "/gr");
  assert.equal(typed("start /gr and more after|"), null, "only on the word the caret is in");
});

test("a / inside a word doesn't open the menu", () => {
  assert.equal(typed("src/app|"), null);
  assert.equal(typed("a/b|"), null);
  assert.equal(typed("see a/|"), null);
});

test("with a command already in the message, another /word doesn't open the menu", () => {
  assert.equal(typed("/compact keep it short /gr|"), null);
  assert.equal(typed("notes /compact and /|"), null);
  assert.equal(typed("/gr| then /compact"), null, "wherever the command sits");
  assert.equal(typed("/compact|"), "/compact", "the command itself still is");
  assert.equal(typed("/foo then /gr|"), "/gr", "an unknown /word isn't a command");
});

test("completing replaces the /word in place and puts the caret after it", () => {
  const text = "fix the login bug /gr and more";
  const at = commandAt(text, 21, NAMES)!;
  assert.deepEqual(completeCommand(text, at, "/grill"), { text: "fix the login bug /grill and more", caret: 24 });
  assert.deepEqual(completeCommand("/cl", commandAt("/cl", 3, NAMES)!, "/clear"), { text: "/clear", caret: 6 });
  assert.deepEqual(completeCommand("a /cl b", commandAt("a /cl b", 5, NAMES)!, ""), { text: "a  b", caret: 2 });
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

test("a command mid-message is coloured, a second /word after it isn't", () => {
  assert.deepEqual(coloured("ask @QA then /grill the plan"), ["@QA", "/grill"]);
  assert.deepEqual(coloured("/compact keep it short /grill"), ["/compact"]);
  assert.deepEqual(coloured("read src/grill"), []);
  const text = "ask @QA then /grill the plan";
  assert.equal(highlightParts(text, NAMES, AGENTS).map((p) => p.text).join(""), text);
});
