// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  comboFromEvent, formatCombo, isReserved, isTyping, loadCombo, normalizeCombo, loadCombos, loadRecent, matchesCombo, nextNeedsYou, saveCombo, saveRecent,
  shortcutAction, switcherAgents,
} from "../src/lib/shortcuts.ts";

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}
type Mods = Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>>;
const key = (code: string, mods: Mods = {}, k = code.replace(/^(Key|Digit)/, "").toLowerCase()) => ({
  key: k, code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods,
});
const defaults = { switcher: "Mod+K", next: "Mod+J" };

test("⌘K and ⌘J on a Mac, Ctrl+K and Ctrl+J elsewhere", () => {
  assert.ok(matchesCombo(key("KeyK", { metaKey: true }), "Mod+K", true));
  assert.ok(!matchesCombo(key("KeyK", { ctrlKey: true }), "Mod+K", true));
  assert.ok(matchesCombo(key("KeyK", { ctrlKey: true }), "Mod+K", false));
  assert.ok(!matchesCombo(key("KeyK", { metaKey: true }), "Mod+K", false));
  assert.deepEqual(shortcutAction(key("KeyJ", { ctrlKey: true }), { combos: defaults, typing: false, afterG: false, mac: false }), { type: "next" });
});

test("a plain key, Shift+key or an extra modifier never matches a combination", () => {
  assert.ok(!matchesCombo(key("KeyK"), "Mod+K", true));
  assert.ok(!matchesCombo(key("KeyK", { shiftKey: true }), "Mod+K", true));
  assert.ok(!matchesCombo(key("KeyK", { metaKey: true, shiftKey: true }), "Mod+K", true));
});

test("recording needs Ctrl, Alt or ⌘ and ignores a lone modifier", () => {
  assert.equal(comboFromEvent(key("KeyP")), null);
  assert.equal(comboFromEvent(key("KeyP", { shiftKey: true })), null);
  assert.equal(comboFromEvent(key("MetaLeft", { metaKey: true }, "Meta")), null);
  assert.equal(comboFromEvent(key("KeyP", { ctrlKey: true, shiftKey: true })), "Ctrl+Shift+P");
  assert.equal(comboFromEvent(key("Digit1", { altKey: true }, "¡")), "Alt+1", "the physical key, not what ⌥ types");
});

test("a recorded combination replaces the old one and reads nicely", () => {
  const c = comboFromEvent(key("KeyJ", { metaKey: true, shiftKey: true }))!;
  const combos = { ...defaults, switcher: c };
  const at = (e: ReturnType<typeof key>) => shortcutAction(e, { combos, typing: false, afterG: false, mac: true });
  assert.deepEqual(at(key("KeyJ", { metaKey: true, shiftKey: true })), { type: "switcher" });
  assert.equal(at(key("KeyK", { metaKey: true })), null, "the old one stops working");
  assert.equal(formatCombo(c, true), "⇧⌘J");
  assert.equal(formatCombo("Mod+K", true), "⌘K");
  assert.equal(formatCombo("Mod+J", false), "Ctrl+J");
});

test("each combination survives a reload; a broken one falls back to its default", () => {
  const s = memory();
  assert.deepEqual(loadCombos(s), defaults);
  saveCombo("next", "Ctrl+Shift+N", s);
  assert.deepEqual(loadCombos(s), { switcher: "Mod+K", next: "Ctrl+Shift+N" });
  saveCombo("switcher", comboFromEvent(key("KeyP", { metaKey: true, shiftKey: true }))!, s);
  assert.equal(loadCombo("switcher", s), "Shift+Meta+P", "⇧⌘P is stored with Shift first and must survive");
  saveCombo("switcher", "Shift+K", s);
  assert.equal(loadCombo("switcher", s), "Mod+K", "Shift alone is typing");
  saveCombo("switcher", "K", s);
  assert.equal(loadCombo("switcher", s), "Mod+K", "never a plain key");
  const blocked = { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } };
  assert.equal(loadCombo("next", blocked), "Mod+J");
});

test("copy, paste, close tab and the like are refused; a saved one falls back to the default", () => {
  const rec = (code: string, mods: Mods) => comboFromEvent(key(code, mods))!;
  assert.ok(isReserved(rec("KeyC", { metaKey: true }), true));
  assert.ok(isReserved(rec("KeyV", { ctrlKey: true }), false));
  assert.ok(isReserved(rec("KeyZ", { metaKey: true, shiftKey: true }), true), "redo");
  assert.ok(isReserved(rec("KeyT", { ctrlKey: true, shiftKey: true }), false), "reopen tab");
  assert.ok(!isReserved(rec("KeyC", { metaKey: true }), false), "⌘C is free where copy is Ctrl+C");
  assert.ok(!isReserved("Mod+K", true) && !isReserved("Mod+J", false));
  const s = memory();
  saveCombo("switcher", "Mod+W", s);
  assert.equal(loadCombo("switcher", s), "Mod+K");
});

test("recording ⌘K on a Mac or Ctrl+K elsewhere stores the default, so Reset hides", () => {
  assert.equal(normalizeCombo("Meta+K", true), "Mod+K");
  assert.equal(normalizeCombo("Ctrl+K", false), "Mod+K");
  assert.equal(normalizeCombo("Ctrl+K", true), "Ctrl+K", "⌃K on a Mac is not ⌘K");
  assert.equal(normalizeCombo("Ctrl+Shift+K", false), "Ctrl+Shift+K");
});

test("text fields count as typing; buttons and checkboxes don't", () => {
  assert.ok(isTyping({ tagName: "TEXTAREA" }));
  assert.ok(isTyping({ tagName: "INPUT", type: "text" }));
  assert.ok(isTyping({ tagName: "INPUT", type: "search" }));
  assert.ok(isTyping({ tagName: "DIV", isContentEditable: true }));
  assert.ok(!isTyping({ tagName: "INPUT", type: "checkbox" }));
  assert.ok(!isTyping({ tagName: "BUTTON" }));
  assert.ok(!isTyping(null));
});

test("⌘K and ⌘J work while typing; g, / and ? don't", () => {
  const typing = (e: ReturnType<typeof key>) => shortcutAction(e, { combos: defaults, typing: true, afterG: false, mac: true });
  assert.deepEqual(typing(key("KeyK", { metaKey: true })), { type: "switcher" });
  assert.deepEqual(typing(key("KeyJ", { metaKey: true })), { type: "next" });
  assert.equal(typing(key("KeyG")), null);
  assert.equal(typing(key("Slash", {}, "/")), null);
  assert.equal(typing(key("Slash", { shiftKey: true }, "?")), null);
  assert.equal(shortcutAction(key("KeyA"), { combos: defaults, typing: true, afterG: true, mac: true }), null, "g then typing a never navigates");
});

test("g then a letter opens a page; / focuses the chat box and ? opens the sheet", () => {
  const at = (e: ReturnType<typeof key>, afterG = false) => shortcutAction(e, { combos: defaults, typing: false, afterG, mac: true });
  assert.deepEqual(at(key("KeyG")), { type: "g" });
  assert.deepEqual(at(key("KeyA"), true), { type: "go", path: "/agents" });
  assert.deepEqual(at(key("KeyI"), true), { type: "go", path: "/issues" });
  assert.deepEqual(at(key("KeyR"), true), { type: "go", path: "/repos" });
  assert.deepEqual(at(key("KeyS"), true), { type: "go", path: "/settings" });
  assert.deepEqual(at(key("KeyM"), true), { type: "go", path: "/simulator" });
  assert.equal(at(key("KeyX"), true), null, "another letter cancels");
  assert.equal(at(key("KeyA")), null, "a alone does nothing");
  assert.deepEqual(at(key("Slash", {}, "/")), { type: "focus-chat" });
  assert.deepEqual(at(key("Slash", { shiftKey: true }, "?")), { type: "sheet" });
  assert.equal(at(key("KeyG", { metaKey: true })), null, "⌘G stays the browser's");
});

test("⌘J goes through the agents in Needs you, in its order, and wraps", () => {
  const items = [{ agent: { id: "dev" } }, { agent: null }, { agent: { id: "lead" } }, { agent: { id: "dev" } }, { agent: { id: "qa" } }];
  assert.equal(nextNeedsYou(items, null), "dev");
  assert.equal(nextNeedsYou(items, "dev"), "lead");
  assert.equal(nextNeedsYou(items, "lead"), "qa", "an agent with two items is visited once");
  assert.equal(nextNeedsYou(items, "qa"), "dev", "wraps");
  assert.equal(nextNeedsYou(items, "someone-else"), "dev");
  assert.equal(nextNeedsYou([{ agent: null }], "dev"), null, "nothing needs you");
  assert.equal(nextNeedsYou([], null), null);
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
