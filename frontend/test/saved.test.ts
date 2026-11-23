import { test } from "node:test";
import assert from "node:assert/strict";
import { defaultTitle, excerpt, linkedMessage, sourceHref, tagOptions } from "../src/lib/saved.ts";

test("the Save form's title is the message's first line, without Markdown's marks", () => {
  assert.equal(defaultTitle("\n\n## How to deploy\n\nRun it."), "How to deploy");
  assert.equal(defaultTitle("- first point\n- second"), "first point");
  assert.equal(defaultTitle("> quoted line"), "quoted line");
  assert.equal(defaultTitle("1. step one"), "step one");
  assert.equal(defaultTitle(""), "");
});

test("a long first line is cut at a word, about 80 characters", () => {
  const t = defaultTitle(`${"word ".repeat(30)}end`);
  assert.ok([...t].length <= 81);
  assert.match(t, /^word( word)*…$/);
});

test("the tag filter offers the suggested tags first, then the others, each once", () => {
  assert.deepEqual(tagOptions([]), ["Tutorial", "Good to know", "Decision"]);
  assert.deepEqual(tagOptions(["decision", "Recipes", " ", "recipes"]), ["Tutorial", "Good to know", "Decision", "Recipes"]);
});

test("the back-link points at the message in its agent's chat, and the chat reads it back", () => {
  const href = sourceHref({ agent_id: "a b", message_id: 42 });
  assert.equal(href, "/agents/a%20b#msg-42");
  assert.equal(linkedMessage("#msg-42"), 42);
  assert.equal(linkedMessage("#perm-1"), null);
  assert.equal(linkedMessage(""), null);
});

test("the list's preview drops Markdown marks and is cut", () => {
  assert.equal(excerpt("# Title\n\n**Bold** and `code`"), "Title Bold and code");
  assert.equal(excerpt("a ".repeat(200), 10), "a a a a a …");
});

test("the preview leaves out a table's separator row and horizontal rules", () => {
  assert.equal(excerpt("| Step | Time |\n|---|:---:|\n| build | 2 min |\n\n---\n\nDone"), "Step Time build 2 min Done");
  assert.equal(excerpt("Step | Time\n--- | ---\nbuild | 2"), "Step Time build 2");
});
