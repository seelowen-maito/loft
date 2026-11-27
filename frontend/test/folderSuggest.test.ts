import { test } from "node:test";
import assert from "node:assert/strict";
import { folderKey, repoCount, stillMatching } from "../src/lib/folderSuggest.ts";

test("arrows move the highlight around the list", () => {
  assert.deepEqual(folderKey("ArrowDown", 0, 3), { move: 1 });
  assert.deepEqual(folderKey("ArrowDown", 2, 3), { move: 0 });
  assert.deepEqual(folderKey("ArrowUp", 0, 3), { move: 2 });
});
test("Tab and Enter pick the highlighted folder", () => {
  assert.deepEqual(folderKey("Tab", 1, 3), { pick: 1 });
  assert.deepEqual(folderKey("Enter", 5, 3), { pick: 2 }, "a list that got shorter picks its last row");
  assert.equal(folderKey("Tab", 0, 3, true), undefined, "Shift-Tab leaves the field");
});
test("Esc closes; other keys and an empty list go to the field", () => {
  assert.equal(folderKey("Escape", 0, 3), "close");
  assert.equal(folderKey("a", 0, 3), undefined);
  assert.equal(folderKey("Tab", 0, 0), undefined);
});
test("repo counts", () => {
  assert.equal(repoCount(0), "no repos");
  assert.equal(repoCount(1), "1 repo");
  assert.equal(repoCount(12), "12 repos");
});

const list = [{ path: "/h/projects" }, { path: "/h/Documents/Projects-old" }, { path: "/h/Documents/proj-x" }];
test("an answer for what's typed shows whole, but the typed folder itself", () => {
  assert.deepEqual(stillMatching("proj", "proj", list), list);
  assert.deepEqual(stillMatching("/h/p", "/h/projects", list), [list[1]], "an older answer, narrowed, without the typed folder");
  assert.deepEqual(stillMatching("/h/projects", "/h/projects", list), list.slice(1));
});
test("an older answer keeps only the folders whose name still starts with what's typed", () => {
  assert.deepEqual(stillMatching("pro", "projects", list), [list[0], list[1]]);
  assert.deepEqual(stillMatching("pro", "proj-", list), [list[2]]);
  assert.deepEqual(stillMatching("pro", "/h/projects/", list), [], "a new folder level waits for its own answer");
});
