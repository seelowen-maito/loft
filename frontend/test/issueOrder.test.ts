// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { compareOpen, isSpec, specProgress } from "../src/lib/issueOrder.ts";

const row = (id: string, o: Partial<{ labels: string[]; priority: "low" | "normal" | "high" | "urgent"; updated_at: string; parent_id: string | null; status: string }> = {}) => ({
  id, labels: [], priority: "normal" as const, updated_at: "2026-10-01T00:00:00Z", parent_id: null, status: "inbox", ...o,
});

test("ready-for-agent issues come first, then priority, then the newest", () => {
  const rows = [
    row("urgent", { priority: "urgent" }),
    row("ready-old", { labels: ["ready-for-agent"], updated_at: "2026-10-01T00:00:00Z" }),
    row("plain-new", { updated_at: "2026-10-09T00:00:00Z" }),
    row("ready-new", { labels: ["bug", "ready-for-agent"], updated_at: "2026-10-05T00:00:00Z" }),
    row("ready-high", { labels: ["ready-for-agent"], priority: "high" }),
  ];
  assert.deepEqual([...rows].sort(compareOpen).map((r) => r.id), ["ready-high", "ready-new", "ready-old", "urgent", "plain-new"]);
});

test("a spec shows its tickets done of all, cancelled ones left out", () => {
  const spec = row("s", { labels: ["spec"] });
  const all = [spec, row("a", { parent_id: "s", status: "done" }), row("b", { parent_id: "s", status: "in_progress" }), row("c", { parent_id: "s" }), row("d", { parent_id: "s", status: "cancelled" }), row("x", { parent_id: "other" })];
  assert.deepEqual(specProgress(spec, all), { done: 1, total: 3 });
  assert.equal(specProgress(row("t", { labels: ["ready-for-agent"] }), all), null, "not a spec");
  assert.equal(specProgress(row("lonely", { labels: ["spec"] }), all), null, "a spec with no tickets yet");
  assert.equal(isSpec(spec), true);
});
