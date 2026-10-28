// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { mergeLoaded, shouldMarkRead, showsUnreadDot, unreadBadge, unreadCount, unreadTitle } from "../src/lib/unread.ts";

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

test("the Agents tab counts every agent with an Unread reply, the open chat included", () => {
  assert.equal(unreadCount([dev, { id: "lead", unread_reply: true }, { id: "rev", unread_reply: false }, { id: "ops" }]), 2);
  assert.equal(unreadCount([]), 0);
});

test("the Agents tab dot shows the count up to nine, then 9+", () => {
  assert.equal(unreadBadge(1), "1");
  assert.equal(unreadBadge(9), "9");
  assert.equal(unreadBadge(10), "9+");
  assert.equal(unreadBadge(42), "9+");
});

test("the browser tab title shows the count only while there are Unread replies", () => {
  assert.equal(unreadTitle(0), "Loft");
  assert.equal(unreadTitle(3), "(3) Loft");
  assert.equal(unreadTitle(12), "(12) Loft");
});

test("updates that arrive while the agent list loads win over it", () => {
  const loaded = [dev, { id: "lead", unread_reply: true }, { id: "rev" }];
  const fresh = new Map<string, { id: string; unread_reply?: boolean } | null>([
    ["dev", { id: "dev", unread_reply: false }],
    ["rev", null],
    ["new", { id: "new", unread_reply: true }],
  ]);
  assert.deepEqual(mergeLoaded(loaded, fresh), [{ id: "dev", unread_reply: false }, { id: "lead", unread_reply: true }, { id: "new", unread_reply: true }]);
  assert.deepEqual(mergeLoaded(loaded, new Map()), loaded);
});
