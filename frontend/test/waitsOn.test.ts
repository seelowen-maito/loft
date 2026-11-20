import { test } from "node:test";
import assert from "node:assert/strict";
import { specsToStartFirst } from "../src/lib/waitsOn.ts";

const w = (blocker: string, delivery: { repo: string; spec: number; started: boolean; issue_id: string | null } | null) => ({ blocker, delivery });

test("offers each unstarted spec that owns an open blocker once; started, unknown and ownerless ones aren't offered", () => {
  const api = { repo: "o/api", spec: 5, started: false, issue_id: "t5" };
  const got = specsToStartFirst([w("o/api#9", api), w("o/api#10", api), w("o/web#2", { repo: "o/web", spec: 1, started: true, issue_id: "t1" }), w("o/x#3", null), w("o/y#4", { repo: "o/y", spec: 2, started: false, issue_id: null })]);
  assert.deepEqual(got, [{ repo: "o/api", spec: 5, issueId: "t5" }]);
  assert.deepEqual(specsToStartFirst(undefined), []);
});
