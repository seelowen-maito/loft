// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { commandAt, completeCommand, enterOnMenu, highlightParts, mentionRanges, messageCommand, splitCommand, type CommandWord } from "../src/lib/commandHighlight.ts";

const NAMES = ["/clear", "/compact", "/grill", "/code", "/resume"];
const span = (text: string, picked?: CommandWord | null) => {
  const c = messageCommand(text, NAMES, picked);
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

/** `name` as picked from the menu at the `/` that starts `at` in `text`. */
const pick = (text: string, at: number, name = text.slice(at).split(/\s/)[0]): CommandWord => ({ start: at, end: at + name.length, word: name });

test("a leading command runs, arguments or not", () => {
  assert.equal(span("/clear"), "/clear");
  assert.equal(span("/grill login bug"), "/grill");
  assert.equal(span("/compact\nkeep the plan"), "/compact");
  assert.equal(span(" /grill"), "/grill");
  assert.equal(span("\n  /grill topic"), "/grill", "after leading whitespace");
});

test("a known command typed by hand mid-message is plain text", () => {
  assert.equal(span("what does /compact do"), null);
  assert.equal(span("why did /clear lose my notes?"), null);
  assert.equal(span("try /grill"), null);
  assert.equal(span("keep notes\n/compact"), null);
  assert.equal(span("/foo then /grill"), null, "only the first word can lead");
});

test("a command picked from the menu mid-message runs while it's still there", () => {
  const text = "the login bug /grill";
  assert.equal(span(text, pick(text, 14)), "/grill");
  assert.equal(span("a /GRILL b", pick("a /GRILL b", 2)), "/GRILL", "in any case");
  assert.equal(span("/compact then /grill", pick("/compact then /grill", 14)), "/compact", "a leading command still wins");
});

test("a picked command edited away, moved or no longer a command is plain text", () => {
  const p = pick("the login bug /grill", 14);
  assert.equal(span("the login bug /gril", p), null, "shortened");
  assert.equal(span("the login bug /grills", p), null, "lengthened");
  assert.equal(span("the login bug", p), null, "deleted");
  assert.equal(span("the big login bug /grill", p), null, "moved by an edit before it");
  assert.equal(span("the login bugx/grill", p), null, "no longer starts a word");
  assert.equal(span("the login bug /grill", pick("the login bug /grill", 14, "/nope")), null, "an unknown name");
});

test("a slash inside a word is no command", () => {
  assert.equal(span("src/grill"), null);
  assert.equal(span(""), null);
});

test("only the leading command counts: a later one is plain text", () => {
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

test("with a command that runs already in the message, another /word doesn't open the menu", () => {
  assert.equal(typed("/compact keep it short /gr|"), null);
  assert.equal(typed("/compact|"), "/compact", "the command itself still is");
  assert.equal(typed("/foo then /gr|"), "/gr", "an unknown /word isn't a command");
  assert.equal(typed("notes /compact and /|"), "/", "a typed mid-message command doesn't run, so it doesn't block");
  const text = "notes /compact and /";
  assert.equal(commandAt(text, text.length, NAMES, pick(text, 6)), null, "a picked one does");
  assert.equal(commandAt("/gr then /compact", 3, NAMES, pick("/gr then /compact", 9)), null, "wherever it sits");
});

test("completing replaces the /word in place and puts the caret after it", () => {
  const text = "fix the login bug /gr and more";
  const at = commandAt(text, 21, NAMES)!;
  assert.deepEqual(completeCommand(text, at, "/grill"), { text: "fix the login bug /grill and more", caret: 24 });
  assert.deepEqual(completeCommand("/cl", commandAt("/cl", 3, NAMES)!, "/clear"), { text: "/clear", caret: 6 });
  assert.deepEqual(completeCommand("a /cl b", commandAt("a /cl b", 5, NAMES)!, ""), { text: "a  b", caret: 2 });
});

/** `splitCommand` of `text` with the /word at `at` picked from the menu. */
const splitPicked = (text: string, at: number) => splitCommand(text, NAMES, pick(text, at));

test("a message splits into its command and the rest, a picked command wherever it sits", () => {
  assert.deepEqual(splitPicked("the login bug /grill", 14), { command: "/grill", rest: "the login bug" });
  assert.deepEqual(splitPicked("keep the API notes /compact", 19), { command: "/compact", rest: "keep the API notes" });
  assert.deepEqual(splitCommand("/grill the login bug", NAMES), { command: "/grill", rest: "the login bug" });
  assert.deepEqual(splitPicked("draft /CLEAR text", 6), { command: "/clear", rest: "draft text" }, "lowercased to run");
  assert.deepEqual(splitCommand("/resume", NAMES), { command: "/resume", rest: "" });
  assert.deepEqual(splitCommand("  /code  ", NAMES), { command: "/code", rest: "" });
});

test("the rest keeps its own line breaks", () => {
  assert.deepEqual(splitPicked("first line\n/compact\nsecond line", 11), { command: "/compact", rest: "first line\nsecond line" });
  assert.deepEqual(splitCommand("/compact\nkeep the plan", NAMES), { command: "/compact", rest: "keep the plan" });
});

test("a second /word stays in the rest as plain text", () => {
  assert.deepEqual(splitCommand("/compact keep /grill notes", NAMES), { command: "/compact", rest: "keep /grill notes" });
  assert.deepEqual(splitPicked("the bug /grill then /clear", 8), { command: "/grill", rest: "the bug then /clear" });
});

test("a message with no command doesn't split", () => {
  assert.equal(splitCommand("fix src/grill please", NAMES), null);
  assert.equal(splitCommand("try /gri", NAMES), null);
  assert.equal(splitCommand("what does /compact do", NAMES), null, "a typed mid-message command is sent as text");
  assert.equal(splitCommand("the bug /gril", NAMES, pick("the bug /grill", 8)), null, "nor one edited after it was picked");
  assert.equal(splitCommand("", NAMES), null);
});

const AGENTS = ["Loft Dev", "Loft Reviewer", "QA", "Web Developer", "Web Dev"];
const coloured = (text: string, picked?: CommandWord | null) =>
  highlightParts(text, NAMES, AGENTS, picked).filter((p) => p.accent).map((p) => p.text);

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

test("only a command that will run is coloured", () => {
  const text = "ask @QA then /grill the plan";
  assert.deepEqual(coloured(text), ["@QA"], "typed by hand mid-message");
  assert.deepEqual(coloured(text, pick(text, 13)), ["@QA", "/grill"], "picked from the menu");
  assert.deepEqual(coloured("ask @QA then /gril the plan", pick(text, 13)), ["@QA"], "picked, then edited away");
  assert.deepEqual(coloured("/compact keep it short /grill"), ["/compact"]);
  assert.deepEqual(coloured("read src/grill"), []);
  assert.equal(highlightParts(text, NAMES, AGENTS, pick(text, 13)).map((p) => p.text).join(""), text);
});

test("Enter on a command typed by hand mid-message picks it instead of running it", () => {
  const text = "explain /compact";
  const at = commandAt(text, text.length, NAMES)!;
  assert.equal(at.word, "/compact", "the menu is open on it");
  assert.equal(enterOnMenu(text, at, "/compact", null), "pick", "the first Enter only picks it");
  assert.equal(splitCommand(text, NAMES), null, "unpicked, the message would go as text");
  assert.equal(enterOnMenu("how do I use /compact", commandAt("how do I use /compact", 21, NAMES)!, "/compact"), "pick");
});

test("Enter on a picked command mid-message sends it, and runs it", () => {
  const text = "explain /compact", picked = pick(text, 8);
  const at = commandAt(text, text.length, NAMES, picked)!;
  assert.equal(enterOnMenu(text, at, "/compact", picked), "send");
  assert.deepEqual(coloured(text, picked), ["/compact"], "highlighted before it runs");
  assert.deepEqual(splitCommand(text, NAMES, picked), { command: "/compact", rest: "explain" });
  const partial = "explain /comp";
  assert.equal(enterOnMenu(partial, commandAt(partial, 13, NAMES)!, "/compact", null), "pick", "a partial word completes");
  assert.equal(enterOnMenu("a /grill b /compact", commandAt("a /grill b /compact", 19, NAMES)!, "/compact", pick("a /grill b /compact", 2)), "pick", "another word picked doesn't count");
});
