// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { cleanLineText, shownCommand } from "../src/lib/format.ts";

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
