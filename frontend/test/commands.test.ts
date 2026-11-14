// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { filterCommands, parseLoop, terminalNote } from "../src/lib/commands.ts";
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

test("/loop takes an interval and a prompt, or stop", () => {
  assert.deepEqual(parseLoop("10m check the deploy"), { interval: "10m", prompt: "check the deploy" });
  assert.deepEqual(parseLoop(" 2H  watch\nthe build "), { interval: "2h", prompt: "watch\nthe build" });
  assert.deepEqual(parseLoop("Stop"), { stop: true });
  for (const bad of ["", "10m", "check the deploy", "10 m go", "m go", "1.5h go"]) assert.equal(parseLoop(bad), null, bad);
});
