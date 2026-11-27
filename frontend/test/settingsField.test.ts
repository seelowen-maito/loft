// Settings: a field's label names its input, the (?) popover is outside the label, and "On this page" follows the scroll.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "MouseEvent", "KeyboardEvent", "FocusEvent"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

let vite: ViteDevServer;
let SettingsField: any;
let cardInView: (ids: string[], tops: Record<string, number | undefined>, line: number, atBottom: boolean, pinned: string | null) => string;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ SettingsField } = await vite.ssrLoadModule("/src/components/SettingsField.tsx"));
  ({ cardInView } = await vite.ssrLoadModule("/src/lib/settingsSpy.ts"));
});
after(() => vite.close());

const click = (el: Element) => act(() => void el.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })));

function render() {
  const host = document.body.appendChild(document.createElement("div"));
  act(() => createRoot(host).render(createElement(SettingsField, { label: "SSH host", hint: "Short.", help: "The long version." }, (id: string) => createElement("input", { id }))));
  return host;
}

test("the label names the input, and the (?) is not inside the label", () => {
  const host = render();
  const input = host.querySelector("input")!;
  const label = host.querySelector("label")!;
  assert.equal(label.htmlFor, input.id);
  assert.equal(label.textContent, "SSH host");
  assert.equal(label.querySelector("button"), null);
  assert.equal(host.querySelector("button")?.getAttribute("aria-label"), "More about SSH host");
});

test("the (?) opens its popover, linked by aria-controls, and Esc and a tap elsewhere close it", () => {
  const host = render();
  const b = host.querySelector("button")!;
  assert.equal(b.getAttribute("aria-expanded"), "false");
  click(b);
  const pop = host.querySelector(`[id="${b.getAttribute("aria-controls")}"]`);
  assert.equal(pop?.textContent, "The long version.");
  assert.equal(b.getAttribute("aria-expanded"), "true");
  act(() => void document.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  assert.equal(host.querySelector("[id]:not(input)"), null);
  click(b);
  act(() => void document.body.dispatchEvent(new dom.window.MouseEvent("pointerdown", { bubbles: true })));
  assert.equal(b.getAttribute("aria-expanded"), "false");
});

test("the popover closes when focus leaves it", () => {
  const host = render();
  const b = host.querySelector("button")!;
  click(b);
  act(() => void b.dispatchEvent(new dom.window.FocusEvent("focusout", { bubbles: true, relatedTarget: host.querySelector("input") })));
  assert.equal(b.getAttribute("aria-expanded"), "false");
});

test("On this page highlights the last card passed, the last one at the bottom, or the one just clicked", () => {
  const ids = ["general", "backups", "push", "claude"];
  const tops = { general: -900, backups: -300, push: 400, claude: 900 };
  assert.equal(cardInView(ids, tops, 96, false, null), "backups");
  assert.equal(cardInView(ids, { ...tops, push: 40 }, 96, false, null), "push");
  assert.equal(cardInView(ids, tops, 96, true, null), "claude", "at the bottom, the last card");
  assert.equal(cardInView(ids, tops, 96, true, "push"), "push", "at the bottom, the card clicked");
  assert.equal(cardInView(ids, { backups: 500 }, 96, false, null), "general", "cards not on the page yet are skipped, and none passed leaves the first");
});
