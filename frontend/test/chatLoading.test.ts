// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { CHAT_LOADING_LINES, chatLoadingLine } from "../src/lib/chatLoading.ts";

test("every line names the agent", () => {
  for (let i = 0; i < CHAT_LOADING_LINES; i++) assert.match(chatLoadingLine("Scout", i), /^Scout /);
});

test("turns move to the next line and wrap around", () => {
  assert.equal(chatLoadingLine("Scout", 0), "Scout is loading your chat…");
  assert.equal(chatLoadingLine("Scout", 0, 1), "Scout is finding your chats…");
  assert.equal(chatLoadingLine("Scout", 0, CHAT_LOADING_LINES), chatLoadingLine("Scout", 0));
  assert.equal(chatLoadingLine("Scout", -1), chatLoadingLine("Scout", CHAT_LOADING_LINES - 1));
});
