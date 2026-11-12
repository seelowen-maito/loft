import { test } from "node:test";
import assert from "node:assert/strict";
import { arrange, boardIssues, deliveryOf, parseBoard, specMeta, specOfAgent, specRef, stageOf, ticketLabel, ticketsOf, withBoard, withOpen, type DeliverySummary } from "../src/lib/specViews.ts";
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
  assert.equal(stageOf(spec({ state: "closed", status: "done" }), del({ state: "settled" })), "shipped", "closed as completed");
  assert.equal(stageOf(spec({ state: "closed", status: "done" }), null), "shipped");
  assert.equal(stageOf(spec({ state: "closed", status: "cancelled" }), del({ state: "ready" })), "dropped", "closed as not planned is not Shipped");
  assert.equal(stageOf(spec({ state: "closed", status: "cancelled" }), null), "dropped");
  assert.equal(stageOf(spec({ state: "closed", status: "cancelled" }), del({ state: "merged" })), "shipped", "a merged Delivery shipped, whatever the issue says");
});

test("ticket states read in words", () => {
  assert.deepEqual(ticketLabel("closed"), { text: "done", tone: "good" });
  assert.equal(ticketLabel("needs_human").tone, "warn");
  assert.equal(ticketLabel("needs_human").text, "Waiting on you");
  assert.equal(ticketLabel("whatever").text, "open");
});

const task = (o: Record<string, unknown>) => ({ labels: ["spec"], state: "open", status: "inbox", parent_id: null, updated_at: "2026-01-01", github: null, ...o }) as unknown as Task;

test("a spec's tickets: its Delivery's, each with its own issue, else its sub-issues", () => {
  const s = task({ id: "s", github: { repo: "o/r", number: 9 } });
  const own = task({ id: "t10", labels: [], github: { repo: "o/r", number: 10 }, parent_id: "s" });
  const d = del({ current: { number: 10, title: "A" }, tickets: [{ number: 10, title: "A", state: "review", skipped: "", agent_id: "a1", verdicts: { qa: "pending" } }, { number: 11, title: "B", state: "open", skipped: "" }] });
  const ts = ticketsOf(s, d, [s, own]);
  assert.deepEqual(ts.map((t) => [t.number, t.taskId, t.current, t.verdicts]), [[10, "t10", true, { qa: "pending" }], [11, null, false, {}]]);
  const kid = task({ id: "k", labels: [], status: "in_progress", parent_id: "s", title: "Kid" });
  assert.deepEqual(ticketsOf(s, null, [s, kid]).map((t) => [t.title, t.state, t.taskId]), [["Kid", "started", "k"]]);
});

test("board options round-trip through the URL and keep the other params", () => {
  const p = withBoard(new URLSearchParams("project=loft&x=1"), { type: "issues", stages: ["delivery", "ready"], hideShipped: true, showDropped: true, byProject: true, sort: "progress", dense: true, needsMe: true });
  assert.equal(p.get("project"), "loft");
  assert.deepEqual(parseBoard(p), { type: "issues", stages: ["delivery", "ready"], hideShipped: true, showDropped: true, byProject: true, sort: "progress", dense: true, needsMe: true });
  assert.equal(withBoard(p, { type: "all", stages: [], hideShipped: false, showDropped: false, byProject: false, sort: "activity", dense: false, needsMe: false }).toString(), "project=loft&x=1", "defaults leave no trace");
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

test("dropped specs stay off the board unless asked for, and a fork's spec and its upstream's stay two cards", () => {
  const done = task({ id: "d", state: "closed", status: "done", updated_at: "2026-01-05", github: { repo: "me/loft", number: 26 } });
  const dropped = task({ id: "x", state: "closed", status: "cancelled", updated_at: "2026-01-06", github: { repo: "me/loft", number: 83 } });
  const upstream = task({ id: "u", state: "closed", status: "done", updated_at: "2026-01-04", github: { repo: "up/loft", number: 265 } });
  const all = [done, dropped, upstream];
  const ids = (o: object) => arrange(all, [], all, { ...parseBoard(new URLSearchParams()), ...o }).map((s) => s.id);
  assert.deepEqual(ids({}), ["d", "u"], "shipped, not the dropped one");
  assert.deepEqual(ids({ showDropped: true }), ["x", "d", "u"]);
  assert.deepEqual(ids({ stages: ["shipped"], showDropped: true }), ["d", "u"]);
  assert.deepEqual(all.map(specRef), ["me/loft#26", "me/loft#83", "up/loft#265"], "each card says which repo it is");
  assert.equal(specRef(task({ project: "web" })), "Loft · web");
});

test("a spec card names its project once: next to a GitHub ref, and only in the ref of a plan kept in Loft", () => {
  assert.deepEqual(specMeta(task({ project: "web", github: { repo: "me/loft", number: 26 } })), { ref: "me/loft#26", project: "web" });
  assert.deepEqual(specMeta(task({ project: "web" })), { ref: "Loft · web", project: null }, "not twice");
  assert.deepEqual(specMeta(task({ project: "", github: { repo: "me/loft", number: 26 } })), { ref: "me/loft#26", project: null }, "no project, nothing added");
});

test("the board's open pane is in the URL, with the spec it came from, and leaves the filters alone", () => {
  const p = new URLSearchParams("view=specs&project=loft");
  assert.equal(withOpen(p, "t1").toString(), "view=specs&project=loft&open=t1");
  assert.equal(withOpen(p, "t1", "s1").toString(), "view=specs&project=loft&open=t1&up=s1");
  assert.equal(withOpen(withOpen(p, "t1", "s1"), "s1").toString(), "view=specs&project=loft&open=s1", "back to the spec drops the arrow");
  assert.equal(withOpen(withOpen(p, "t1", "s1"), null).toString(), "view=specs&project=loft", "closing leaves the board as it was");
});

test("on the board a spec is an agent's when it owns the spec or builds its Delivery", () => {
  const ds = [del({ spec: 9, agent_id: "x" }), del({ spec: 10, agent_id: "y" })];
  const delivered = spec({ gh: ["o/r", 9], agent_id: null });
  const owned = spec({ gh: ["o/r", 10], agent_id: "x" });
  const neither = spec({ gh: ["o/r", 10], agent_id: "z" });
  const none = spec({ gh: ["o/r", 11], agent_id: null });
  assert.ok(specOfAgent(delivered, ds, "x"), "no agent on the spec, but its Delivery's agent is x");
  assert.ok(specOfAgent(owned, ds, "x"), "owned by x, though y delivers it");
  assert.ok(!specOfAgent(neither, ds, "x"));
  assert.ok(!specOfAgent(none, ds, "x"));
  assert.ok([delivered, owned, neither, none].every((s) => specOfAgent(s, ds, "")), "no agent filter: every spec");
});

test("the type filter is in the URL, All being the default", () => {
  assert.equal(parseBoard(new URLSearchParams()).type, "all");
  assert.equal(parseBoard(new URLSearchParams("type=issues")).type, "issues");
  assert.equal(parseBoard(new URLSearchParams("type=nope")).type, "all");
  assert.equal(withBoard(new URLSearchParams("project=loft"), { type: "specs" }).toString(), "project=loft&type=specs");
  assert.equal(withBoard(new URLSearchParams("type=specs&project=loft"), { type: "all" }).toString(), "project=loft");
});

test("the Issues column holds the issues that are no spec and no spec's ticket, open first then the newest", () => {
  const t = (o: object) => ({ id: "x", state: "open", status: "inbox", labels: [], description: "", parent_id: null, updated_at: "2026-01-01", github: null, ...o }) as unknown as Task;
  const standalone = t({ id: "a", updated_at: "2026-01-02", github: { repo: "o/r", number: 1 } });
  const newer = t({ id: "b", updated_at: "2026-01-03" });
  const aSpec = t({ id: "s", labels: ["Spec"], github: { repo: "o/r", number: 9 } });
  const child = t({ id: "c", parent_id: "s" });
  const named = t({ id: "d", description: "## Parent\n\n#9\n" });
  const inDelivery = t({ id: "e", github: { repo: "o/r", number: 11 } });
  const done = t({ id: "f", state: "closed", status: "done", updated_at: "2026-01-09" });
  const ds = [del({ spec: 9, tickets: [{ number: 11, title: "T", state: "open", skipped: "" }] })];
  const all = [standalone, newer, aSpec, child, named, inDelivery, done];
  const o = parseBoard(new URLSearchParams());
  assert.deepEqual(boardIssues(all, ds, true, o, false).map((x) => x.id), ["b", "a"], "no specs, no spec tickets, no closed");
  assert.deepEqual(boardIssues(all, ds, true, o, true).map((x) => x.id), ["b", "a", "f"], "closed ones last");
  assert.deepEqual(boardIssues(all.map((x) => (x.id === "a" ? { ...x, status: "blocked" } : x)), ds, true, { ...o, needsMe: true }, false).map((x) => x.id), ["a"]);
  assert.deepEqual(boardIssues(all, ds, false, o, true), [], "nothing is decided before the Deliveries load, so no ticket flickers in");
  assert.deepEqual(boardIssues(all, [], true, o, false).map((x) => x.id), ["b", "a", "e"], "loaded with no Deliveries: all stay");
});

test("an open ticket waiting on another spec's ticket says which", () => {
  assert.deepEqual(ticketLabel("open", ["o/ext#9", "o/ext#10"]), { text: "waiting on o/ext#9, o/ext#10", tone: "warn" });
  assert.equal(ticketLabel("open", []).text, "open");
  assert.equal(ticketLabel("started", ["o/ext#9"]).text, "building");
});
