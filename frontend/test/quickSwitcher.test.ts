// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { comboFromEvent, DEFAULT_COMBO, formatCombo, loadCombo, loadRecent, matchesCombo, saveCombo, saveRecent, switcherAgents } from "../src/lib/quickSwitcher.ts";

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}
const key = (code: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}, k = code.replace(/^Key/, "").toLowerCase()) => ({
  key: k, code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods,
});

test("⌘K opens it on a Mac and Ctrl+K elsewhere", () => {
  assert.ok(matchesCombo(key("KeyK", { metaKey: true }), DEFAULT_COMBO, true));
  assert.ok(!matchesCombo(key("KeyK", { ctrlKey: true }), DEFAULT_COMBO, true));
  assert.ok(matchesCombo(key("KeyK", { ctrlKey: true }), DEFAULT_COMBO, false));
  assert.ok(!matchesCombo(key("KeyK", { metaKey: true }), DEFAULT_COMBO, false));
});

test("a plain key, Shift+key or an extra modifier never matches", () => {
  assert.ok(!matchesCombo(key("KeyK"), DEFAULT_COMBO, true));
  assert.ok(!matchesCombo(key("KeyK", { shiftKey: true }), DEFAULT_COMBO, true));
  assert.ok(!matchesCombo(key("KeyK", { metaKey: true, shiftKey: true }), DEFAULT_COMBO, true));
});

test("recording needs Ctrl, Alt or ⌘ and ignores a lone modifier", () => {
  assert.equal(comboFromEvent(key("KeyP")), null);
  assert.equal(comboFromEvent(key("KeyP", { shiftKey: true })), null);
  assert.equal(comboFromEvent(key("MetaLeft", { metaKey: true }, "Meta")), null);
  assert.equal(comboFromEvent(key("KeyP", { ctrlKey: true, shiftKey: true })), "Ctrl+Shift+P");
  assert.equal(comboFromEvent(key("Digit1", { altKey: true }, "¡")), "Alt+1", "the physical key, not what ⌥ types");
});

test("a recorded combination matches itself and reads nicely", () => {
  const c = comboFromEvent(key("KeyJ", { metaKey: true, shiftKey: true }))!;
  assert.ok(matchesCombo(key("KeyJ", { metaKey: true, shiftKey: true }), c, true));
  assert.ok(!matchesCombo(key("KeyK", { metaKey: true }), c, true), "the old one stops working");
  assert.equal(formatCombo(c, true), "⇧⌘J");
  assert.equal(formatCombo(DEFAULT_COMBO, true), "⌘K");
  assert.equal(formatCombo(DEFAULT_COMBO, false), "Ctrl+K");
});

test("the combination survives a reload, and a broken one falls back to ⌘K", () => {
  const s = memory();
  assert.equal(loadCombo(s), DEFAULT_COMBO);
  saveCombo("Ctrl+Shift+P", s);
  assert.equal(loadCombo(s), "Ctrl+Shift+P");
  saveCombo("K", s);
  assert.equal(loadCombo(s), DEFAULT_COMBO, "never a plain key");
  const blocked = { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } };
  assert.equal(loadCombo(blocked), DEFAULT_COMBO);
});

const agents = [
  { id: "lead", name: "Loft Lead", title: "Plans", group: "Loft", project: "loft" },
  { id: "dev", name: "Loft Dev", title: "Rust and TypeScript", group: "Loft", project: "loft" },
  { id: "mobile", name: "Loft mobile dev", title: "iOS", group: "Loft", project: "loft" },
  { id: "dine", name: "Dineout Dev", title: "Backend", group: "Dineout", project: "api", projects: ["api", "web"] },
];

test("with nothing typed the most recently opened come first, then the rest in order", () => {
  const s = memory();
  saveRecent("mobile", s);
  saveRecent("dine", s);
  saveRecent("mobile", s);
  assert.deepEqual(loadRecent(s), ["mobile", "dine"]);
  assert.deepEqual(switcherAgents(agents, "", loadRecent(s)).map((a) => a.id), ["mobile", "dine", "lead", "dev"]);
});

test("typing filters by name, title, group and project", () => {
  const ids = (q: string) => switcherAgents(agents, q, []).map((a) => a.id);
  assert.deepEqual(ids("ios"), ["mobile"]);
  assert.deepEqual(ids("dineout"), ["dine"]);
  assert.deepEqual(ids("web"), ["dine"], "any of its projects");
  assert.deepEqual(ids("loft dev"), ["dev", "mobile"], "every word, names starting with it first");
  assert.deepEqual(ids("DEV"), ids("dev"), "case-insensitive");
  assert.deepEqual(ids("dev"), ["dev", "mobile", "dine"]);
  assert.deepEqual(ids("d"), ["dine", "lead", "dev", "mobile"], "a name starting with it comes first");
  assert.deepEqual(ids("nobody"), []);
});
