// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { loadCollapsed, toggleCollapsed } from "../src/lib/collapsed.ts";

const KEY = "loft:collapsed-issue-projects";

/** An in-memory localStorage; a fresh `loadCollapsed` on it stands in for a reload. */
function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

test("every project starts expanded", () => {
  assert.deepEqual(loadCollapsed(KEY, memory()), []);
});

test("a header click collapses the group, a second one expands it", () => {
  const s = memory();
  const folded = toggleCollapsed(KEY, [], "loft", s);
  assert.deepEqual(folded, ["loft"]);
  assert.deepEqual(toggleCollapsed(KEY, folded, "loft", s), []);
});

test("the state is remembered per project across a reload", () => {
  const s = memory();
  let c = toggleCollapsed(KEY, loadCollapsed(KEY, s), "loft", s);
  c = toggleCollapsed(KEY, c, "imole", s);
  c = toggleCollapsed(KEY, c, "imole", s);
  assert.deepEqual(loadCollapsed(KEY, s), ["loft"], "loft stays collapsed, imole is expanded again");
  assert.ok(!loadCollapsed(KEY, s).includes("new-project"), "a project seen for the first time is expanded");
});

test("the Issues tab and the Agents list keep separate state", () => {
  const s = memory();
  toggleCollapsed(KEY, [], "loft", s);
  assert.deepEqual(loadCollapsed("loft:collapsed-projects", s), []);
});

test("unreadable or unavailable storage falls back to all expanded", () => {
  const s = memory();
  s.setItem(KEY, "{not json");
  assert.deepEqual(loadCollapsed(KEY, s), []);
  s.setItem(KEY, '{"loft":true}');
  assert.deepEqual(loadCollapsed(KEY, s), []);
  const blocked = { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } };
  assert.deepEqual(loadCollapsed(KEY, blocked), []);
  assert.deepEqual(toggleCollapsed(KEY, [], "loft", blocked), ["loft"], "it still folds for this visit");
});
