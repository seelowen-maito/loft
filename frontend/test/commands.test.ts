// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { filterCommands, terminalNote } from "../src/lib/commands.ts";
import type { AgentCommand } from "../src/lib/api.ts";

const c = (name: string, description = "", tag: AgentCommand["tag"] = "passthrough", reason: string | null = null): AgentCommand => ({ name, description, tag, reason });
const LIST = [c("/clear", "Clear the conversation", "loft"), c("/review", "Review a pull request"), c("/code-review", "Review the diff"), c("/doctor", "Check the install", "terminal", "Terminal only."), c("/ship", "Ship the branch; reviews first")];
const names = (word: string) => filterCommands(LIST, word).map((x) => x.name);

test("a bare slash lists every command in order", () => {
  assert.deepEqual(names("/"), LIST.map((x) => x.name));
});

test("names that start with the word come before names that contain it, then descriptions", () => {
  assert.deepEqual(names("/rev"), ["/review", "/code-review", "/ship"]);
  assert.deepEqual(names("/INSTALL"), ["/doctor"]);
});

test("a word nothing matches lists nothing", () => {
  assert.deepEqual(names("/zzz"), []);
});

test("a terminal command's note says why it can't run, with a default", () => {
  assert.equal(terminalNote(LIST[3]), "/doctor can't run here. Terminal only.");
  assert.equal(terminalNote(c("/x", "", "terminal")), "/x can't run here. Claude Code runs it only in its own terminal app.");
});
