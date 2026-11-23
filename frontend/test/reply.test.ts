import { test } from "node:test";
import assert from "node:assert/strict";
import { authorLabel, copiedToBox, jumpStep, quotable, quoteGone, quoteOf, replyLabel, replyOf } from "../src/lib/reply.ts";

const msg = (role: string, content: string, meta: object | null = null) => ({ id: 3, role, content, meta });

test("a quote names who wrote the message: the agent, another agent, or the user as You", () => {
  assert.equal(quoteOf(msg("assistant", "hi"), "Dev").author, "Dev");
  assert.equal(quoteOf(msg("user", "hi", { from_agent: { id: "q", name: "QA" } }), "Dev").author, "QA");
  const mine = quoteOf(msg("user", "hi"), "Dev");
  assert.equal(mine.author, null);
  assert.equal(authorLabel(mine), "You");
});

test("a quote's excerpt is one line, cut with an ellipsis when long", () => {
  assert.equal(quoteOf(msg("assistant", "Which port\n\n  should it use?"), "Dev").excerpt, "Which port should it use?");
  const long = quoteOf(msg("assistant", "x".repeat(300)), "Dev").excerpt;
  assert.equal(long, `${"x".repeat(200)}…`);
});

test("a sent message's reply_to reads as its quote; anything else is no quote", () => {
  assert.deepEqual(replyOf({ reply_to: { id: 9, author: null, excerpt: "Ship it", role: "user" } }), { id: 9, author: null, excerpt: "Ship it" });
  for (const meta of [null, {}, { reply_to: null }, { reply_to: { id: "9" } }]) assert.equal(replyOf(meta), null, JSON.stringify(meta));
});

test("only chat messages offer Reply: not run markers, tools or thinking", () => {
  for (const [role, kind] of [["user", "text"], ["assistant", "text"]]) assert.ok(quotable({ role, kind }), kind);
  for (const [role, kind] of [["system", "run_start"], ["system", "clear"], ["assistant", "thinking"], ["assistant", "tool_use"], ["tool", "tool_result"], ["assistant", "error"]]) {
    assert.ok(!quotable({ role, kind }), `${role} ${kind}`);
  }
});

test("a clicked quote scrolls to a shown message, shows the items before /clear, or says it's gone", () => {
  assert.equal(jumpStep(true, 12), "flash");
  assert.equal(jumpStep(false, 12), "show-earlier");
  // Older than what the chat loaded: nothing hidden to show, so the note shows right away.
  assert.equal(jumpStep(false, 0), "gone");
});

test("only the gone-quote refusal drops the composer's quote", () => {
  assert.equal(quoteGone("The quoted message isn't in this chat."), true);
  assert.equal(quoteGone("Only chat messages can be quoted."), false);
  assert.equal(quoteGone("empty message"), false);
});

test("the footer's Reply names the author", () => {
  assert.equal(replyLabel(msg("user", "hi"), "Dev"), "Reply to your message");
  assert.equal(replyLabel(msg("assistant", "hi"), "Dev"), "Reply to Dev");
  assert.equal(replyLabel(msg("user", "hi", { from_agent: { id: "l", name: "Loft Lead" } }), "Dev"), "Reply to Loft Lead");
});

test("Copy to box keeps what the box already holds", () => {
  const m = msg("user", "bump the build");
  assert.equal(copiedToBox("", m), "bump the build");
  assert.equal(copiedToBox("  ", m), "bump the build");
  assert.equal(copiedToBox("draft", m), "draft\n\nbump the build");
});

test("Copy to box on a task brief copies what the bubble shows, without Loft's tag and instructions", () => {
  const brief = msg("user", "[Task 0b6f6c1e-3c1a-4f0e-9c55-2a7d1f0b9e11] Fix the Mac icon.\n\n[Report progress on this task with update_issue.]", { task_id: "0b6f6c1e-3c1a-4f0e-9c55-2a7d1f0b9e11" });
  assert.equal(copiedToBox("", brief), "Fix the Mac icon.");
});
