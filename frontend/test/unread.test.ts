// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { shouldMarkRead, showsUnreadDot } from "../src/lib/unread.ts";

const dev = { id: "dev", unread_reply: true };

test("the open chat is marked read while the tab is visible", () => {
  assert.equal(shouldMarkRead(dev, "dev", true), true);
});

test("a reply that arrives in a hidden tab stays unread until the tab is visible", () => {
  assert.equal(shouldMarkRead(dev, "dev", false), false);
  assert.equal(shouldMarkRead(dev, "dev", true), true);
});

test("other chats and agents without an Unread reply aren't marked", () => {
  assert.equal(shouldMarkRead(dev, "lead", true), false);
  assert.equal(shouldMarkRead({ id: "dev", unread_reply: false }, "dev", true), false);
  assert.equal(shouldMarkRead({ id: "dev" }, "dev", true), false);
  assert.equal(shouldMarkRead(null, null, true), false);
});

test("the roster dot shows for each agent with an Unread reply except the open chat", () => {
  assert.equal(showsUnreadDot(dev, "lead"), true);
  assert.equal(showsUnreadDot(dev, null), true);
  assert.equal(showsUnreadDot(dev, "dev"), false);
  assert.equal(showsUnreadDot({ id: "lead" }, "dev"), false);
});
