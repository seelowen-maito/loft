// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { chosenRows, ticksAfter, canCheck, cleanState, filterClean, inBatches, initialTicks, selectable, cleanLineText, gitCheckText, oneLine, shownCommand } from "../src/lib/format.ts";

const item = (o: Partial<{ label: string; blocked: string; opens: boolean }> = {}) => ({
  agent: "a", path: "/s/a/loft", rel: "loft", name: "loft", kind: "clone" as const, bytes: 1, changed: "", command: "rm -rf /s/a/loft",
  label: "", note: "", warning: "", ticked: false, blocked: "", opens: false, ...o,
});

test("log lines read past tense, like their header", () => {
  const line = { agent: "a", path: "/s/a/old", rel: "old", bytes: 2_200_000_000, error: null, command: "rm -rf /s/a/old" };
  assert.equal(cleanLineText(line), "Removed /s/a/old (2.2 GB)");
  assert.equal(cleanLineText({ ...line, error: "Permission denied" }), "failed /s/a/old: Permission denied");
});

test("Simulator device and process lines say what was done to which one", () => {
  const line = { agent: "a", path: "/sim/U", rel: "Loft Dev 93543007", bytes: 2_000_000_000, error: null, command: "xcrun simctl delete U", done: "Deleted Simulator device" };
  assert.equal(cleanLineText(line), "Deleted Simulator device Loft Dev 93543007 (2.0 GB)");
  assert.equal(cleanLineText({ ...line, rel: "Loft Dev (Loft)", bytes: 0, done: "Shut down Simulator device" }), "Shut down Simulator device Loft Dev (Loft)");
  assert.equal(cleanLineText({ ...line, bytes: 0, error: "busy" }), "failed: Deleted Simulator device Loft Dev 93543007: busy");
  assert.equal(cleanLineText({ ...line, path: "pid 42", rel: "node vite --port 20070", bytes: 0, command: "kill 42", done: "Stopped process" }), "Stopped process node vite --port 20070");
});

test("only items that can be removed show their command", () => {
  assert.equal(shownCommand(item(), 0), "rm -rf /s/a/loft");
  assert.equal(shownCommand(item({ label: "main clone of loft", opens: true }), 0), "", "a main clone");
  assert.equal(shownCommand(item({ blocked: "Working" }), 0), "", "a working agent's item");
  assert.equal(shownCommand(item({ label: "kept", opens: true }), 1), "rm -rf /s/a/loft", "a folder inside a main clone");
});

test("a name keeps to its one line in Copy commands", () => {
  assert.equal(oneLine("Deleted agent \nrm -r ~\r\t\u0085x"), "Deleted agent rm -r ~x");
  assert.equal(oneLine("Loft Dev"), "Loft Dev");
});

test("only a clone or worktree at the top of a workspace gets Check", () => {
  assert.equal(canCheck(item(), 0), true);
  assert.equal(canCheck({ ...item(), kind: "worktree" }, 0), true);
  assert.equal(canCheck({ ...item(), kind: "folder" }, 0), false);
  assert.equal(canCheck(item(), 1), false);
});

test("a check reads as its sign and lines", () => {
  assert.equal(gitCheckText({ state: "safe", lines: ["Clean, all pushed: safe to delete", "the agent clones it again on its next ticket"] }), "✅ Clean, all pushed: safe to delete · the agent clones it again on its next ticket");
  assert.equal(gitCheckText({ state: "unsafe", lines: ["2 commits not pushed (main)", "Uncommitted changes (1 file)"] }), "⚠️ 2 commits not pushed (main) · Uncommitted changes (1 file)");
  assert.equal(gitCheckText({ state: "unknown", lines: ["Can't check: unusual git config"] }), "❔ Can't check: unusual git config");
});

test("status says Working, why a row is locked, or Deletable", async () => {
  const { cleanStatus, cleanType } = await import("../src/lib/format.ts");
  assert.deepEqual(cleanStatus(item({ blocked: "Working" }), 0), { text: "Working", locked: true });
  assert.deepEqual(cleanStatus(item({ blocked: "symlink" }), 0), { text: "Locked: symlink", locked: true });
  assert.deepEqual(cleanStatus(item({ opens: true, note: "Clean, all pushed: safe to delete" }), 0), { text: "Deletable", locked: false }, "a main clone git says is safe");
  assert.deepEqual(cleanStatus(item({ opens: true, blocked: "1 commit not pushed (main)" }), 0), { text: "Locked: 1 commit not pushed (main)", locked: true });
  assert.deepEqual(cleanStatus(item({ label: "main clone of loft", opens: true }), 0), { text: "Kept: main clone of loft", locked: true });
  assert.deepEqual(cleanStatus(item(), 0), { text: "Deletable", locked: false });
  assert.equal(cleanType(item()), "git clone");
  assert.equal(cleanType({ ...item(), kind: "build" }), "build output");
});

test("the table sorts by size, agent or last change, either way", async () => {
  const { sortClean } = await import("../src/lib/format.ts");
  const a = { ...item(), agent: "b", name: "x", bytes: 5, changed: "2026-01-02T00:00:00Z" };
  const b = { ...item(), agent: "a", name: "y", bytes: 9, changed: "2026-01-01T00:00:00Z" };
  const c = { ...item(), agent: "a", name: "z", bytes: 1, changed: "2026-01-03T00:00:00Z" };
  const names = (s: "size" | "agent" | "changed", desc: boolean) => sortClean([a, b, c], { by: s, desc }, (id) => id).map((i) => i.name).join("");
  assert.equal(names("size", true), "yxz");
  assert.equal(names("size", false), "zxy");
  assert.equal(names("agent", false), "yzx");
  assert.equal(names("changed", true), "zxy");
});

test("an empty or invalid last change sorts as the oldest, not as NaN", async () => {
  const { sortClean } = await import("../src/lib/format.ts");
  const at = (name: string, changed: string) => ({ ...item(), name, bytes: 1, changed });
  const rows = [at("a", "2026-01-02T00:00:00Z"), at("b", ""), at("c", "not a date"), at("d", "2026-01-01T00:00:00Z")];
  const names = (desc: boolean) => sortClean(rows, { by: "changed", desc }, (id) => id).map((i) => i.name).join("");
  assert.equal(names(false), "bcda");
  assert.equal(names(true), "adbc");
});

test("a locked reason splits into its lines, and the filters keep one agent, one type, or what can be ticked", async () => {
  const { reasonLines, filterClean } = await import("../src/lib/format.ts");
  assert.deepEqual(reasonLines("2 commits not pushed (main) · Stashed changes · Uncommitted changes (3 files)"), ["2 commits not pushed (main)", "Stashed changes", "Uncommitted changes (3 files)"]);
  assert.deepEqual(reasonLines(""), []);
  const a = { ...item(), agent: "a", name: "x" };
  const b = { ...item(), agent: "b", name: "y", kind: "build", blocked: "Working" };
  const all = { agent: "", type: "", deletable: false };
  assert.equal(filterClean([a, b], all).length, 2);
  assert.deepEqual(filterClean([a, b], { ...all, agent: "b" }).map((i) => i.name), ["y"]);
  assert.deepEqual(filterClean([a, b], { ...all, type: "build output" }).map((i) => i.name), ["y"]);
  assert.deepEqual(filterClean([a, b], { ...all, deletable: true }).map((i) => i.name), ["x"]);
});

test("a clone is Deletable only after a clean check; select all skips unchecked and locked rows", () => {
  const clone = item({});
  const folder = { ...item(), path: "/s/a/out", kind: "build" as const };
  const working = { ...item(), path: "/s/b/x", blocked: "Working" };
  assert.equal(cleanState(clone, 0).text, "Check git first");
  assert.equal(cleanState(clone, 0, { state: "unsafe", lines: [] }).text, "Check git first");
  assert.equal(cleanState(clone, 0, { state: "safe", lines: [] }).text, "Deletable · checked");
  assert.equal(cleanState(folder, 0).text, "Deletable");
  const checks = new Map<string, any>();
  assert.deepEqual(selectable([clone, folder, working], checks).map((i) => i.path), ["/s/a/out"]);
  checks.set(clone.path, { state: "safe", lines: [] });
  checks.set(working.path, { state: "safe", lines: [] });
  assert.deepEqual(selectable([clone, folder, working], checks).map((i) => i.path), ["/s/a/loft", "/s/a/out"]);
  // Only what the filters show is offered.
  const shown = filterClean([clone, folder, { ...folder, path: "/s/z", agent: "z" }], { agent: "a", type: "", deletable: false });
  assert.equal(selectable(shown, checks).length, 2);
});

test("inBatches runs at most `limit` at once and reports progress", async () => {
  let live = 0, peak = 0;
  const seen: number[] = [];
  await inBatches([1, 2, 3, 4, 5, 6, 7], 3, async () => { peak = Math.max(peak, ++live); await new Promise((r) => setTimeout(r, 5)); live--; }, (n) => seen.push(n));
  assert.equal(peak, 3);
  assert.deepEqual(seen, [1, 2, 3, 4, 5, 6, 7]);
});

test("Clean starts with the rows the API ticks, but not clones, locked rows or what the API leaves unticked", () => {
  const out = { ...item(), path: "/s/a/dev.log", kind: "file" as const, ticked: true };
  const open = { ...item(), path: "/s/a/scratch/413", kind: "folder" as const, ticked: false };
  const clone = { ...item(), ticked: true };
  const working = { ...out, path: "/s/b/x.log", blocked: "Working" };
  assert.deepEqual([...initialTicks([out, open, clone, working])], ["/s/a/dev.log"]);
});

test("only a safe check leaves a row ticked, so a re-check that finds trouble unticks it", () => {
  assert.equal(ticksAfter({ state: "safe", lines: [] }), true);
  assert.equal(ticksAfter({ state: "unsafe", lines: [] }), false);
  assert.equal(ticksAfter({ state: "unknown", lines: [] }), false);
});

test("Delete selected takes only ticked rows the filters show, folders opened inside them included", () => {
  const a = { ...item(), path: "/s/a/x" };
  const hidden = { ...item(), path: "/s/b/y" };
  const child = { ...item(), path: "/s/a/x/out", rel: "x/out" };
  const ticked = new Set(["/s/a/x", "/s/b/y", "/s/a/x/out"]);
  assert.deepEqual(chosenRows([a], new Map([[a.path, [child]]]), ticked).map((i) => i.path), ["/s/a/x", "/s/a/x/out"]);
  assert.equal(chosenRows([a], new Map(), ticked).some((i) => i.path === hidden.path), false);
});
