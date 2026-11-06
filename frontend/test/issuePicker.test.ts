import { test } from "node:test";
import assert from "node:assert/strict";
import { ATTACH, createdByMe, filterIssues } from "../src/lib/issueFilter.ts";

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

test("a bare number finds the GitHub issue with that number, and the text that names it", () => {
  // Spec #288's own title and body don't say 288; its tickets' bodies do ("Parent #288").
  const spec = { ...(issue("Tech debt", 288) as object), description: "" } as never;
  const ticket = { ...(issue("A ticket", 290) as object), description: "## Parent\n\n#288" } as never;
  const found = filterIssues([spec, ticket, ...issues], "288", "all").map((t: { title: string }) => t.title);
  assert.deepEqual(found, ["Tech debt", "A ticket"]);
  assert.deepEqual(titles("42", "all"), ["Retry cap", "Fix #42 later"]);
});

test("text searches titles and labels, and combines with the origin", () => {
  assert.deepEqual(titles("retry", "all"), ["Retry cap", "Retry docs"]);
  assert.deepEqual(titles("docs", "github"), ["Retry docs"]);
  assert.deepEqual(titles("retry", "loft"), []);
});

test("Created by me keeps the user's Loft issues and the GitHub issues their token's account opened", () => {
  const t = (created_by: string, author?: string) => ({ created_by, github: author === undefined ? null : { author } }) as never;
  assert.equal(createdByMe(t("user"), "octo-me"), true, "a Loft issue the user created");
  assert.equal(createdByMe(t("agent-1"), "octo-me"), false, "one an agent created");
  assert.equal(createdByMe(t("github", "Octo-Me"), "octo-me"), true, "opened on GitHub by the user");
  assert.equal(createdByMe(t("agent-1", "octo-me"), "octo-me"), true, "published by Loft with the user's token");
  assert.equal(createdByMe(t("github", "someone"), "octo-me"), false, "opened by someone else");
  assert.equal(createdByMe(t("user", "someone"), "octo-me"), false, "the user's Loft issue that tracks someone else's GitHub issue");
  assert.equal(createdByMe(t("github", ""), null), false, "without the login, a mirrored issue counts by who made it in Loft");
  assert.equal(createdByMe(t("user", ""), null), true);
});
