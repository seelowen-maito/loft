import { test } from "node:test";
import assert from "node:assert/strict";
import type { PullRequest } from "../src/lib/api.ts";
import { mergeOrder, namedPrs, parseRefs } from "../src/lib/prRefs.ts";

const pr = (number: number, more: Partial<PullRequest> = {}): PullRequest =>
  ({ id: `p${number}`, agent_id: "a", slug: "octo/loft", number, state: "open", head_ref: `b${number}`, base_ref: "main", checked_at: null, ...more }) as PullRequest;

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
