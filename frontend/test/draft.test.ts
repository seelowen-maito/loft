// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  EMPTY_DRAFT, clearAnswerDrafts, clearDraft, loadAnswerDraft, loadDraft, saveAnswerDraft, saveDraft,
} from "../src/lib/draft.ts";

/** An in-memory localStorage; a fresh `loadDraft` on it stands in for a reload. */
function memory() {
  const m = new Map<string, string>();
  return {
    m,
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
  };
}

const draft = { text: "half a thought", images: ["a.png", "b.jpg"], task: "t1" };
const NO_ANSWER = { text: "", images: [] };

test("with nothing saved an agent's draft is empty", () => {
  assert.deepEqual(loadDraft("dev", memory()), EMPTY_DRAFT);
});

test("a draft is still there after a reload, with its images and task", () => {
  const s = memory();
  saveDraft("dev", draft, s);
  assert.deepEqual(loadDraft("dev", s), draft);
});

test("one agent's draft never shows in another's composer", () => {
  const s = memory();
  saveDraft("dev", draft, s);
  assert.deepEqual(loadDraft("reviewer", s), EMPTY_DRAFT);
  saveDraft("reviewer", { text: "other", images: [], task: null }, s);
  assert.equal(loadDraft("dev", s).text, "half a thought");
  assert.equal(loadDraft("reviewer", s).text, "other");
});

test("clearing one agent's draft leaves the others", () => {
  const s = memory();
  saveDraft("dev", draft, s);
  saveDraft("reviewer", draft, s);
  clearDraft("dev", s);
  assert.deepEqual(loadDraft("dev", s), EMPTY_DRAFT);
  assert.deepEqual(loadDraft("reviewer", s), draft);
});

test("an empty draft takes no storage", () => {
  const s = memory();
  saveDraft("dev", draft, s);
  saveDraft("dev", EMPTY_DRAFT, s);
  assert.equal(s.m.size, 0);
});

test("a corrupt draft loads as empty", () => {
  const s = memory();
  s.setItem("loft:draft:dev", "{not json");
  assert.deepEqual(loadDraft("dev", s), EMPTY_DRAFT);
  s.setItem("loft:draft:dev", JSON.stringify({ text: 3, images: ["a.png", 4], task: 5 }));
  assert.deepEqual(loadDraft("dev", s), { text: "", images: ["a.png"], task: null });
});

test("grill answer drafts are kept per question and round, with their images", () => {
  const s = memory();
  saveAnswerDraft("g1", 2, "q1", { text: "maybe Postgres", images: ["shot.png"] }, s);
  saveAnswerDraft("g1", 2, "q2", { text: "", images: ["logo.png"] }, s);
  assert.deepEqual(loadAnswerDraft("g1", 2, "q1", s), { text: "maybe Postgres", images: ["shot.png"] });
  assert.deepEqual(loadAnswerDraft("g1", 2, "q2", s), { text: "", images: ["logo.png"] }, "an image alone is a draft");
  assert.deepEqual(loadAnswerDraft("g1", 3, "q1", s), NO_ANSWER, "a later round starts blank");
  assert.deepEqual(loadAnswerDraft("g2", 2, "q1", s), NO_ANSWER, "another grill starts blank");
  assert.deepEqual(loadDraft("g1", s), EMPTY_DRAFT, "answers aren't a composer draft");
  saveAnswerDraft("g1", 2, "q1", NO_ANSWER, s);
  assert.equal(s.m.has("loft:draft:grill:g1:2:q1"), false, "an emptied answer takes no storage");
});

test("a saved answer or a closed round deletes its drafts", () => {
  const s = memory();
  for (const q of ["q1", "q2", "q3"]) saveAnswerDraft("g1", 1, q, { text: q, images: [] }, s);
  clearAnswerDrafts("g1", 1, ["q1"], s);
  assert.deepEqual(loadAnswerDraft("g1", 1, "q1", s), NO_ANSWER);
  assert.equal(loadAnswerDraft("g1", 1, "q2", s).text, "q2");
  clearAnswerDrafts("g1", 1, ["q2", "q3"], s);
  assert.equal(s.m.size, 0);
});

test("storage that throws never breaks the composer", () => {
  const boom = () => { throw new Error("blocked"); };
  const blocked = { getItem: boom, setItem: boom, removeItem: boom };
  saveDraft("dev", draft, blocked);
  clearDraft("dev", blocked);
  assert.deepEqual(loadDraft("dev", blocked), EMPTY_DRAFT);
  saveAnswerDraft("g1", 1, "q1", { text: "a", images: [] }, blocked);
  clearAnswerDrafts("g1", 1, ["q1"], blocked);
  assert.deepEqual(loadAnswerDraft("g1", 1, "q1", blocked), NO_ANSWER);
});
