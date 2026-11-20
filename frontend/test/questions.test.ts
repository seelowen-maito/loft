// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { answerFor, answersOf, questionsOf, toggle, typeOther } from "../src/lib/questions.ts";

const empty = { chosen: [], other: "" };

test("a single-choice question keeps one pick, and Other replaces it", () => {
  let p = toggle(empty, "S3", false);
  p = toggle(p, "B2", false);
  assert.deepEqual(p.chosen, ["B2"]);
  assert.deepEqual(toggle(p, "B2", false).chosen, [], "picking it again clears it");
  assert.deepEqual(typeOther(p, "R2", false), { chosen: [], other: "R2" });
  assert.deepEqual(toggle({ chosen: [], other: "R2" }, "S3", false), { chosen: ["S3"], other: "" });
});

test("a multi-choice question collects picks and Other together", () => {
  let p = toggle(empty, "api", true);
  p = toggle(p, "loft", true);
  p = typeOther(p, " pos ", true);
  const qs = questionsOf({ questions: [{ question: "Which projects?", multiSelect: true }] });
  assert.deepEqual(answersOf(qs, { 0: p }), { "Which projects?": "api, loft, pos" });
  assert.deepEqual(toggle(p, "api", true).chosen, ["loft"]);
});

test("answers wait until every question has one", () => {
  const qs = questionsOf({ questions: [{ question: "Store?" }, { question: "Region?" }, { header: "no text" }] });
  assert.equal(qs.length, 2, "a question without text is skipped");
  assert.equal(answersOf(qs, { 0: { chosen: ["S3"], other: "" } }), null);
  assert.deepEqual(answersOf(qs, { 0: { chosen: ["S3"], other: "" }, 1: { chosen: [], other: "eu" } }), { "Store?": "S3", "Region?": "eu" });
  assert.deepEqual(questionsOf(undefined), []);
});

test("questions named __proto__ or constructor answer and read back like any other", () => {
  const qs = questionsOf({ questions: [{ question: "__proto__" }, { question: "constructor" }, { question: "toString" }] });
  const picks = { 0: { chosen: ["a"], other: "" }, 1: { chosen: [], other: "b" }, 2: { chosen: ["c"], other: "" } };
  const answers = answersOf(qs, picks)!;
  assert.deepEqual(Object.keys(answers), ["__proto__", "constructor", "toString"]);
  assert.equal(Object.getPrototypeOf(answers), Object.prototype, "the answer didn't replace the prototype");
  assert.deepEqual(JSON.parse(JSON.stringify(answers)), JSON.parse('{"__proto__":"a","constructor":"b","toString":"c"}'));
  assert.equal(answerFor(answers, "__proto__"), "a");
  // Not answered: nothing inherited is read as an answer.
  for (const q of ["__proto__", "constructor", "toString", "hasOwnProperty"]) assert.equal(answerFor({}, q), null, q);
  assert.equal(answerFor({ x: { not: "a string" } }, "x"), null);
});

test("only string text from the agent reaches the card", () => {
  const [q] = questionsOf({ questions: [{ question: "Q?", header: { x: 1 }, options: [{ label: "ok", description: 5 }, { label: { x: 1 } }, null] }] });
  assert.deepEqual(q, { question: "Q?", header: undefined, multiSelect: false, options: [{ label: "ok", description: undefined }] });
});
