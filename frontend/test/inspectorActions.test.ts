// The Inspector's Danger zone and memory list in jsdom, with `api` stubbed: each button shows it's working, then its error.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });
// The memory list listens for server events; nothing is sent here.
Object.assign(globalThis, { EventSource: class { close() {} } });

let vite: ViteDevServer;
let api: Record<string, (...a: unknown[]) => Promise<unknown>>;
let DangerZone: (p: object) => ReactNode;
let MemoryTab: (p: object) => ReactNode;
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ api } = await vite.ssrLoadModule("/src/lib/api.ts"));
  ({ DangerZone, MemoryTab } = await vite.ssrLoadModule("/src/pages/agents/Inspector.tsx"));
});
after(() => vite.close());

const agent = { id: "a1", name: "Dev", builtin: false };
function render(el: ReactNode) {
  const div = document.body.appendChild(document.createElement("div"));
  const root = createRoot(div);
  act(() => root.render(el));
  return div;
}
const button = (div: Element, text: string) => [...div.querySelectorAll("button")].find((b) => b.textContent?.trim() === text || b.getAttribute("aria-label") === text)!;
/** A request that waits until `fail` is called, then rejects with `message`. */
function slowFail(message: string) {
  let fail!: () => void;
  const p = new Promise((_, no) => (fail = () => no(new Error(message))));
  return { call: () => p, fail };
}

test("Clear it and Delete agent show they're working, then their error", async () => {
  const clear = slowFail("clear failed");
  const del = slowFail("delete failed");
  api.workspaceSize = () => Promise.resolve({ bytes: 0 });
  api.clearMessages = clear.call;
  api.deleteAgent = del.call;
  const div = render(h(DangerZone, { agent, onCleared: () => {} }));
  act(() => button(div, "Clear chat history").click());
  act(() => button(div, "Clear it").click());
  assert.equal(div.querySelector('button[aria-busy="true"]')?.textContent, "Clearing…");
  await act(async () => clear.fail());
  assert.equal(div.querySelector('[role="alert"]')?.textContent, "clear failed");

  act(() => button(div, "Delete agent").click());
  act(() => button(div, "Delete Dev").click());
  assert.equal(div.querySelector('button[aria-busy="true"]')?.textContent, "Deleting…");
  await act(async () => del.fail());
  assert.ok([...div.querySelectorAll('[role="alert"]')].some((e) => e.textContent === "delete failed" || e.textContent === "clear failed"));
});

test("adding and forgetting a memory show they're working, then their error", async () => {
  const add = slowFail("add failed");
  const forget = slowFail("forget failed");
  api.memories = () => Promise.resolve([{ id: 7, title: "Use pnpm", content: "", tags: "", created_at: new Date().toISOString() }]);
  api.addMemory = add.call;
  api.deleteMemory = forget.call;
  const div = render(h(MemoryTab, { agent }));
  await act(async () => {});
  const input = div.querySelector<HTMLInputElement>("input")!;
  const setValue = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setValue.call(input, "Run tests first");
    input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  });
  act(() => div.querySelector("form")!.dispatchEvent(new dom.window.Event("submit", { bubbles: true, cancelable: true })));
  assert.equal(div.querySelector('button[aria-busy="true"]')?.getAttribute("aria-label"), "Adding memory…");
  await act(async () => add.fail());
  assert.equal(div.querySelector('[role="alert"]')?.textContent, "add failed");

  act(() => button(div, "Forget").click());
  const busy = div.querySelector('button[aria-busy="true"]')!;
  assert.equal(busy.getAttribute("aria-label"), "Forgetting…");
  assert.ok(busy.querySelector(".animate-spin"));
  await act(async () => forget.fail());
  assert.ok([...div.querySelectorAll('[role="alert"]')].some((e) => e.textContent === "forget failed"));
});
