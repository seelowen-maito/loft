// The navbar hover shared by session, week, RAM and temperature: instant on a mouse, a toggle on touch.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "MouseEvent"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

let vite: ViteDevServer;
let HoverTip: (p: { tip: string; children: string }) => unknown;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ HoverTip } = await vite.ssrLoadModule("/src/components/HoverTip.tsx"));
});
after(() => vite.close());

// jsdom has no PointerEvent: a MouseEvent carrying a pointerType is what React reads.
const pointer = (type: string, pointerType: string) => Object.assign(new dom.window.MouseEvent(type, { bubbles: true }), { pointerType });
const fire = (el: Element, e: Event) => act(() => void el.dispatchEvent(e));

function render() {
  const host = document.body.appendChild(document.createElement("div"));
  act(() => createRoot(host).render(createElement(HoverTip as any, { tip: "Memory used 9.0 of 16 GB" }, "RAM 56%")));
  return host.querySelector("button")!;
}
const tip = (b: Element) => b.querySelector('[role="tooltip"]')?.textContent;

test("a mouse shows the tip as it enters and hides it as it leaves, with no delay or title attribute", () => {
  const b = render();
  assert.equal(b.hasAttribute("title"), false);
  assert.equal(tip(b), undefined);
  fire(b, pointer("pointerover", "mouse"));
  assert.equal(tip(b), "Memory used 9.0 of 16 GB");
  fire(b, pointer("pointerout", "mouse"));
  assert.equal(tip(b), undefined);
});

test("a tap opens it, a second tap closes it, and a tap elsewhere closes it", () => {
  const b = render();
  const tap = () => { fire(b, pointer("pointerdown", "touch")); fire(b, new dom.window.MouseEvent("click", { bubbles: true })); };
  tap();
  assert.equal(tip(b), "Memory used 9.0 of 16 GB");
  tap();
  assert.equal(tip(b), undefined);
  tap();
  fire(document.body, pointer("pointerdown", "touch"));
  assert.equal(tip(b), undefined);
});
