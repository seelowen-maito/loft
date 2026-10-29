import { test } from "node:test";
import assert from "node:assert/strict";
import { filterIssues } from "../src/lib/issueFilter.ts";
import { ATTACH } from "../src/lib/attach.ts";

const issue = (title: string, number?: number, labels: string[] = []) =>
  ({ id: title, title, description: "", labels, owner: "", github: number ? { number } : null }) as never;
const issues = [issue("Retry cap", 42), issue("Retry docs", 7, ["docs"]), issue("Local only"), issue("Fix #42 later")];
const titles = (q: string, source: "all" | "loft" | "github") => filterIssues(issues, q, source).map((t: { title: string }) => t.title);

test("the action is called Attach an issue", () => {
  assert.equal(ATTACH, "Attach an issue");
});

test("All lists Loft-only and GitHub-origin issues together; the filter splits them", () => {
  assert.deepEqual(titles("", "all"), ["Retry cap", "Retry docs", "Local only", "Fix #42 later"]);
  assert.deepEqual(titles("", "loft"), ["Local only", "Fix #42 later"]);
  assert.deepEqual(titles("", "github"), ["Retry cap", "Retry docs"]);
});

test("#42 finds the issue mirroring GitHub #42, and only it", () => {
  assert.deepEqual(titles("#42", "all"), ["Retry cap"]);
  assert.deepEqual(titles("#42", "loft"), []);
  assert.deepEqual(titles("#99", "all"), []);
});

test("text searches titles and labels, and combines with the origin", () => {
  assert.deepEqual(titles("retry", "all"), ["Retry cap", "Retry docs"]);
  assert.deepEqual(titles("docs", "github"), ["Retry docs"]);
  assert.deepEqual(titles("retry", "loft"), []);
});
