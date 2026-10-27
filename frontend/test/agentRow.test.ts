// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { byActivity, cardsPerRow, lastActivity } from "../src/lib/agentRow.ts";

const NOW = Date.parse("2026-10-27T12:00:00Z");

function s(name: string, o: { running?: boolean; run?: string; reply?: string; created?: string } = {}) {
  return {
    name,
    agent: { running: o.running, last_run_at: o.run ?? null, created_at: o.created ?? "2026-01-01T00:00:00Z" },
    last_reply: o.reply ? { at: o.reply } : null,
  };
}

test("an agent's activity is its latest run or reply, else its creation", () => {
  assert.equal(lastActivity(s("a", { run: "2026-10-20T00:00:00Z", reply: "2026-10-21T00:00:00Z" }), NOW), Date.parse("2026-10-21T00:00:00Z"));
  assert.equal(lastActivity(s("b", { run: "2026-10-22T00:00:00Z", reply: "2026-10-21T00:00:00Z" }), NOW), Date.parse("2026-10-22T00:00:00Z"));
  assert.equal(lastActivity(s("c", { created: "2026-10-01T00:00:00Z" }), NOW), Date.parse("2026-10-01T00:00:00Z"));
  assert.equal(lastActivity(s("d", { running: true, run: "2026-10-01T00:00:00Z" }), NOW), NOW, "a running agent is active now");
});

test("cards are ordered by most recent activity, ties keep their order", () => {
  const list = [
    s("old", { run: "2026-10-01T00:00:00Z" }),
    s("never"),
    s("recent", { reply: "2026-10-26T00:00:00Z" }),
    s("running", { running: true }),
    s("never-too"),
  ];
  assert.deepEqual(byActivity(list, NOW).map((x) => x.name), ["running", "recent", "old", "never", "never-too"]);
  assert.equal(list[0].name, "old", "the input is left as it was");
});

test("one row holds 1 card on a phone, 2 from 640px, 3 from 1280px", () => {
  assert.equal(cardsPerRow(390), 1);
  assert.equal(cardsPerRow(639), 1);
  assert.equal(cardsPerRow(640), 2);
  assert.equal(cardsPerRow(1279), 2);
  assert.equal(cardsPerRow(1280), 3);
  assert.equal(cardsPerRow(2560), 3);
});
