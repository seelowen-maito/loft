import { test } from "node:test";
import assert from "node:assert/strict";
import type { PullRequest } from "../src/lib/api.ts";
import { mergeOrder, namedPrs, parseRefs, refLabel } from "../src/lib/prRefs.ts";
import { isOpen, mergeability } from "../src/lib/mergeability.ts";

const pr = (number: number, more: Partial<PullRequest> = {}): PullRequest =>
  ({ id: `p${number}`, agent_id: "a", slug: "octo/loft", number, state: "open", mergeable: true, mergeable_state: "clean", head_ref: `b${number}`, base_ref: "main", checked_at: null, ...more }) as PullRequest;

test("a message's PR tags, links included, in the order it names them", () => {
  const refs = parseRefs("Merge #628, then octo/api#12 and https://github.com/Octo/Loft/pull/634. Again #628.");
  assert.deepEqual(refs, [{ slug: null, number: 628 }, { slug: "octo/api", number: 12 }, { slug: "octo/loft", number: 634 }]);
});

test("not tags: inside a word, a colour, a path, a version, code", () => {
  assert.deepEqual(parseRefs("abc#12 #fff a/b/c#4 #12abc #12/x file.md#3 #3.5 `#7` ```\n#8\n```"), []);
});

test("bare numbers mean the agent's project; a PR tracked twice counts once, its freshest copy", () => {
  const prs = [pr(628), pr(634, { id: "old", checked_at: "2026-11-01" }), pr(634, { id: "new", checked_at: "2026-11-02" }), pr(5, { slug: "octo/api" })];
  assert.deepEqual(namedPrs("#634 then #628, #5 and octo/api#5", "loft", prs).map((p) => p.id), ["new", "p628", "p5"]);
  assert.deepEqual(namedPrs("#5", "", prs).map((p) => p.id), ["p5"], "no project: the only PR with that number");
  assert.deepEqual(namedPrs("#7", "loft", prs), [], "untracked");
});

test("merge order: open as named, a stacked PR waits for its base, merged and closed go last", () => {
  const rows = mergeOrder([pr(1, { state: "merged" }), pr(3, { base_ref: "b2" }), pr(2), pr(4, { state: "closed" }), pr(5, { base_ref: "b1" })]);
  assert.deepEqual(rows.map((r) => [r.pr.number, r.after]), [[3, 2], [2, null], [5, null], [1, null], [4, null]]);
});

test("once its base merges, a stacked PR no longer waits", () => {
  const rows = mergeOrder([pr(2, { state: "merged" }), pr(3, { base_ref: "b2" })]);
  assert.deepEqual(rows.map((r) => [r.pr.number, r.after]), [[3, null], [2, null]]);
});

test("a stacked PR can't be merged before its base, even when GitHub says it's clean", () => {
  const [stacked] = mergeOrder([pr(3, { base_ref: "b2" }), pr(2)]);
  assert.equal(stacked.after, 2);
  const m = mergeability(stacked.pr, stacked.after);
  assert.equal(m.canMerge, false);
  assert.equal(m.label, "After #2");
  assert.equal(mergeability(stacked.pr).canMerge, true, "the same PR on its own can merge");
});

test("a PR Loft hasn't read yet counts as open everywhere", () => {
  assert.ok(isOpen(pr(1, { state: "" })));
  assert.deepEqual(mergeOrder([pr(1, { state: "merged" }), pr(2, { state: "" })]).map((r) => r.pr.number), [2, 1]);
});

test("rows name PRs from another repo in full", () => {
  const prs = [pr(628), pr(12, { slug: "octo/api" })];
  assert.deepEqual(prs.map((p) => refLabel(p, prs, "loft")), ["#628", "octo/api#12"]);
  assert.deepEqual(prs.map((p) => refLabel(p, prs, "")), ["#628", "octo/api#12"], "no project: the first PR's repo is home");
  assert.deepEqual(prs.map((p) => refLabel(p, prs, "api")), ["octo/loft#628", "#12"]);
});
