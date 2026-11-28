// The New agent dialog's role picker in jsdom: hovering a line never scrolls the list, the keyboard does, and the footer
// stays out of the scrolling list at a fixed height, so the list can't move under the mouse (#499).
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Element", "Node", "Event", "KeyboardEvent", "MouseEvent"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

// Which lines were scrolled into view, by their row number.
const scrolled: string[] = [];
dom.window.Element.prototype.scrollIntoView = function (this: HTMLElement) {
  scrolled.push(this.dataset.row ?? "?");
};

let vite: ViteDevServer;
let RoleField: (p: object) => ReactNode;
let lib: typeof import("../src/lib/newAgent.ts");
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  // After jsdom is set up: react-dom checks the DOM's events once, as it loads.
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ RoleField } = await vite.ssrLoadModule("/src/pages/agents/RolePicker.tsx"));
  lib = (await vite.ssrLoadModule("/src/lib/newAgent.ts")) as typeof lib;
});
after(() => vite.close());

const template = (key: string, role: string, about: string) => ({ key, name: key, role, summary: `${key} line`, about, personality: "", prompt: `${key} prompt`, model: "sonnet", interval_minutes: 0 });
const TEMPLATES = [
  ...["lead", "dev", "code_reviewer", "qa", "security"].map((r) => template(r, r, `What ${r} does.`)),
  template("deps", "custom", "Upgrades dependencies in small, safe steps, after reading the changelogs, and flags known vulnerabilities. ".repeat(3)),
  ...["docs", "triage", "ci"].map((k) => template(k, "custom", `What ${k} does.`)),
];

function open() {
  const { team, solo } = lib.rolePicks(TEMPLATES, (r) => r);
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  act(() => root.render(h(RoleField, { form: lib.EMPTY_FORM, current: lib.BLANK_CUSTOM, team, solo, error: null, onPick: () => {} })));
  act(() => (document.querySelector('button[aria-haspopup="dialog"]') as HTMLElement).click());
  const panel = document.querySelector('[role="dialog"][aria-label="Choose a role"]')!;
  assert.ok(panel, "the panel opens");
  return { panel, rows: [...panel.querySelectorAll<HTMLElement>('[role="option"]')], unmount: () => act(() => root.unmount()) };
}
const lit = (panel: Element) => panel.querySelector('[aria-selected="true"]')?.getAttribute("data-row");
const hover = (el: Element) => act(() => void el.dispatchEvent(new dom.window.MouseEvent("mousemove", { bubbles: true })));
const press = (key: string) =>
  act(() => void (document.activeElement ?? document.body).dispatchEvent(new dom.window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));

test("hovering a line highlights it without scrolling the list; the keys scroll the line they highlight", () => {
  scrolled.length = 0;
  const { panel, rows, unmount } = open();
  assert.equal(rows.length, 10);
  // Opening shows the current line, blank Custom.
  assert.equal(lit(panel), "5");
  assert.deepEqual(scrolled, ["5"]);
  scrolled.length = 0;

  for (const i of [6, 9, 8, 9, 6]) {
    hover(rows[i]);
    assert.equal(lit(panel), String(i));
  }
  assert.deepEqual(scrolled, [], "the mouse never scrolls");

  press("ArrowDown");
  assert.equal(lit(panel), "7");
  press("0");
  assert.equal(lit(panel), "9");
  press("1");
  assert.equal(lit(panel), "0");
  assert.deepEqual(scrolled, ["7", "9", "0"]);
  unmount();
});

test("the footer sits below the list, outside it, a fixed height whatever the line says", () => {
  const { panel, rows, unmount } = open();
  const list = panel.querySelector('[role="listbox"]')!;
  assert.match(list.className, /\boverflow-y-auto\b/, "the list scrolls by itself");
  const footer = panel.lastElementChild!;
  assert.ok(!list.contains(footer) && list.nextElementSibling === footer);
  assert.doesNotMatch(footer.className, /\bsticky\b/);
  const about = footer.querySelector("p")!;
  // Deps (the long one) and Docs: lines 6 and 7, after the 5 team lines and blank Custom.
  for (const i of [6, 7]) {
    hover(rows[i]);
    assert.equal(about.textContent, TEMPLATES[i - 1].about);
    assert.equal(about.title, TEMPLATES[i - 1].about, "whatever the clamp cuts can still be read");
    assert.match(about.className, /(^| )line-clamp-4 h-\[5\.5em\]( |$)/, "four lines on a phone");
    assert.match(about.className, /(^| )sm:line-clamp-3 sm:h-\[4\.125em\]( |$)/, "three lines wider");
  }
  unmount();
});
