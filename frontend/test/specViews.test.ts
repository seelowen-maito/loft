import { test } from "node:test";
import assert from "node:assert/strict";
import { deliveryOf, stageOf, ticketLabel, type DeliverySummary } from "../src/lib/specViews.ts";
import type { Task } from "../src/lib/api.ts";

const spec = (o: Partial<Task> & { gh?: [string, number] }): Task =>
  ({ id: "s1", state: "open", github: o.gh ? { repo: o.gh[0], number: o.gh[1] } : null, ...o }) as unknown as Task;
const del = (o: Partial<DeliverySummary>): DeliverySummary => ({ state: "running", started: true, agent_id: null, agent_name: null, pr: null, repo: "o/r", spec: 9, spec_id: "", local: false, tickets: [], ...o });

test("a GitHub spec finds its Delivery by repo and number, a plan kept in Loft by its issue", () => {
  const a = del({ spec: 9 }), b = del({ spec: 10 }), l = del({ repo: "loft:web", spec: 1, local: true, spec_id: "s1" });
  assert.equal(deliveryOf(spec({ gh: ["o/r", 10] }), [a, b, l]), b);
  assert.equal(deliveryOf(spec({}), [a, b, l]), l);
  assert.equal(deliveryOf(spec({ gh: ["o/r", 11] }), [a, b]), null);
});

test("the stage follows the Delivery and the issue", () => {
  const open = spec({});
  assert.equal(stageOf(open, null), "planned");
  assert.equal(stageOf(open, del({ state: "planned" })), "planned");
  for (const s of ["running", "final_review", "final_fixes", "needs_human"]) assert.equal(stageOf(open, del({ state: s })), "delivery", s);
  assert.equal(stageOf(open, del({ state: "ready" })), "ready");
  assert.equal(stageOf(open, del({ state: "settled" })), "ready");
  assert.equal(stageOf(open, del({ state: "merged" })), "shipped");
  assert.equal(stageOf(spec({ state: "closed" }), del({ state: "settled" })), "shipped", "closed on GitHub");
});

test("ticket states read in words", () => {
  assert.deepEqual(ticketLabel("closed"), { text: "done", tone: "good" });
  assert.equal(ticketLabel("needs_human").tone, "warn");
  assert.equal(ticketLabel("whatever").text, "open");
});
