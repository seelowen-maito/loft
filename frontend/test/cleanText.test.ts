// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { canCheck, cleanLineText, gitCheckText, oneLine, shownCommand } from "../src/lib/format.ts";

const item = (o: Partial<{ label: string; blocked: string; opens: boolean }> = {}) => ({
  agent: "a", path: "/s/a/loft", rel: "loft", name: "loft", kind: "clone" as const, bytes: 1, changed: "", command: "rm -rf /s/a/loft",
  label: "", note: "", warning: "", ticked: false, blocked: "", opens: false, ...o,
});

test("log lines read past tense, like their header", () => {
  const line = { agent: "a", path: "/s/a/old", rel: "old", bytes: 2_200_000_000, error: null, command: "rm -rf /s/a/old" };
  assert.equal(cleanLineText(line), "Removed /s/a/old (2.2 GB)");
  assert.equal(cleanLineText({ ...line, error: "Permission denied" }), "failed /s/a/old: Permission denied");
});

test("only items that can be removed show their command", () => {
  assert.equal(shownCommand(item(), 0), "rm -rf /s/a/loft");
  assert.equal(shownCommand(item({ label: "main clone of loft", opens: true }), 0), "", "a main clone");
  assert.equal(shownCommand(item({ blocked: "Working" }), 0), "", "a working agent's item");
  assert.equal(shownCommand(item({ label: "kept", opens: true }), 1), "rm -rf /s/a/loft", "a folder inside a main clone");
});

test("a name keeps to its one line in Copy commands", () => {
  assert.equal(oneLine("Deleted agent \nrm -r ~\r\t\u0085x"), "Deleted agent rm -r ~x");
  assert.equal(oneLine("Loft Dev"), "Loft Dev");
});

test("only a clone or worktree at the top of a workspace gets Check", () => {
  assert.equal(canCheck(item(), 0), true);
  assert.equal(canCheck({ ...item(), kind: "worktree" }, 0), true);
  assert.equal(canCheck({ ...item(), kind: "folder" }, 0), false);
  assert.equal(canCheck(item(), 1), false);
});

test("a check reads as its sign and lines", () => {
  assert.equal(gitCheckText({ state: "safe", lines: ["Clean, all pushed: safe to delete", "the agent clones it again on its next ticket"] }), "✅ Clean, all pushed: safe to delete · the agent clones it again on its next ticket");
  assert.equal(gitCheckText({ state: "unsafe", lines: ["2 commits not pushed (main)", "Uncommitted changes (1 file)"] }), "⚠️ 2 commits not pushed (main) · Uncommitted changes (1 file)");
  assert.equal(gitCheckText({ state: "unknown", lines: ["Can't check: unusual git config"] }), "❔ Can't check: unusual git config");
});
