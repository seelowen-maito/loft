import { test } from "node:test";
import assert from "node:assert/strict";
import { arrange, deliveryOf, laneOf, parseBoard, stageOf, ticketLabel, ticketsOf, withBoard, type DeliverySummary } from "../src/lib/specViews.ts";
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

const task = (o: Record<string, unknown>) => ({ labels: ["spec"], state: "open", status: "inbox", parent_id: null, updated_at: "2026-01-01", github: null, ...o }) as unknown as Task;

test("a spec's tickets: its Delivery's, each with its own issue, else its sub-issues", () => {
  const s = task({ id: "s", github: { repo: "o/r", number: 9 } });
  const own = task({ id: "t10", labels: [], github: { repo: "o/r", number: 10 }, parent_id: "s" });
  const d = del({ current: { number: 10, title: "A" }, tickets: [{ number: 10, title: "A", state: "review", skipped: "", verdicts: { qa: "pending" } }, { number: 11, title: "B", state: "open", skipped: "" }] });
  const ts = ticketsOf(s, d, [s, own]);
  assert.deepEqual(ts.map((t) => [t.number, t.taskId, t.current, t.verdicts]), [[10, "t10", true, { qa: "pending" }], [11, null, false, {}]]);
  const kid = task({ id: "k", labels: [], status: "in_progress", parent_id: "s", title: "Kid" });
  assert.deepEqual(ticketsOf(s, null, [s, kid]).map((t) => [t.title, t.state, t.taskId]), [["Kid", "started", "k"]]);
});

test("tickets fall in three lanes", () => {
  assert.deepEqual(["open", "elsewhere", "started", "review", "needs_human", "closed"].map(laneOf), ["open", "open", "doing", "doing", "doing", "done"]);
});

test("board options round-trip through the URL and keep the other params", () => {
  const p = withBoard(new URLSearchParams("project=loft&specView=b1"), { stages: ["delivery", "ready"], hideShipped: true, byProject: true, sort: "progress", dense: true, needsMe: true });
  assert.equal(p.get("project"), "loft");
  assert.deepEqual(parseBoard(p), { stages: ["delivery", "ready"], hideShipped: true, byProject: true, sort: "progress", dense: true, needsMe: true });
  assert.equal(withBoard(p, { stages: [], hideShipped: false, byProject: false, sort: "activity", dense: false, needsMe: false }).toString(), "project=loft&specView=b1", "defaults leave no trace");
  assert.deepEqual(parseBoard(new URLSearchParams("stage=nope,ready")).stages, ["ready"]);
});

test("the board keeps the specs the options ask for, most recent or furthest along first", () => {
  const a = task({ id: "a", updated_at: "2026-01-02", github: { repo: "o/r", number: 1 } }), b = task({ id: "b", updated_at: "2026-01-03", github: { repo: "o/r", number: 2 } }), c = task({ id: "c", state: "closed", updated_at: "2026-01-04", github: { repo: "o/r", number: 3 } });
  const ds = [del({ spec: 1, state: "ready" }), del({ spec: 2, state: "running", tickets: [{ number: 1, title: "x", state: "closed", skipped: "" }, { number: 2, title: "y", state: "open", skipped: "" }] })];
  const ids = (o: object) => arrange([a, b, c], ds, [a, b, c], { ...parseBoard(new URLSearchParams()), ...o }).map((s) => s.id);
  assert.deepEqual(ids({}), ["c", "b", "a"]);
  assert.deepEqual(ids({ hideShipped: true }), ["b", "a"]);
  assert.deepEqual(ids({ stages: ["ready"] }), ["a"]);
  assert.deepEqual(ids({ needsMe: true }), ["a"], "ready to merge needs you");
  assert.deepEqual(ids({ sort: "progress" }), ["b", "c", "a"]);
});
