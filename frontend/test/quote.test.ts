// Renders the reply quotes in jsdom. Vite loads the component, so the .tsx compiles as it does in the app.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "MouseEvent"]) {
  Object.assign(globalThis, { [k]: (dom.window as any)[k] });
}

let vite: ViteDevServer;
let Quote: Record<string, (p: object) => unknown>;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  Quote = await vite.ssrLoadModule("/src/pages/agents/Quote.tsx");
});
after(() => vite.close());

function render(name: string, props: object) {
  const host = document.body.appendChild(document.createElement("div"));
  const root = createRoot(host);
  act(() => root.render(createElement(Quote[name] as any, props)));
  return { host, unmount: () => act(() => root.unmount()) };
}

const click = (el: Element) => act(() => void el.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })));
const html = { id: 5, author: "Dev", excerpt: `<img src=x onerror="alert(1)"><b>bold</b> **md**` };

test("quoted text is plain text in the chip and on the sent message", () => {
  for (const name of ["QuoteChip", "QuoteBlock"]) {
    const { host, unmount } = render(name, { q: html, onRemove: () => {}, onJump: () => {} });
    assert.equal(host.querySelector("img, b, strong"), null, name);
    assert.ok(host.textContent!.includes(html.excerpt), name);
    unmount();
  }
});

test("the chip names who is quoted and ✕ removes it", () => {
  let removed = 0;
  const { host, unmount } = render("QuoteChip", { q: { id: 1, author: null, excerpt: "Ship it" }, onRemove: () => removed++ });
  assert.ok(host.textContent!.includes("Replying to You"));
  click(host.querySelector('[aria-label="Remove the quote"]')!);
  assert.equal(removed, 1);
  unmount();
});

test("clicking a sent message's quote jumps to the original", () => {
  const jumps: number[] = [];
  const { host, unmount } = render("QuoteBlock", { q: html, onJump: (id: number) => jumps.push(id) });
  click(host.querySelector("button")!);
  assert.deepEqual(jumps, [5]);
  unmount();
});
