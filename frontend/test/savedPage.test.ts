// The Save form and the Saved page in jsdom (#677), against a fake API.
import { after, before, beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "localStorage", "matchMedia"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

// Every request the page sends, and what the fake API answers.
let calls: { method: string; url: string; body: any }[] = [];
let answer: (url: string) => { status?: number; body: unknown };
globalThis.fetch = (async (url: string, init?: RequestInit) => {
  calls.push({ method: init?.method ?? "GET", url, body: init?.body ? JSON.parse(String(init.body)) : null });
  const a = answer(url);
  return new Response(JSON.stringify(a.body), { status: a.status ?? 200 });
}) as typeof fetch;
beforeEach(() => (calls = []));

let vite: ViteDevServer;
let SaveDialog: (p: object) => ReactNode;
let SavedPage: () => ReactNode;
let MemoryRouter: (p: object) => ReactNode;
let Routes: (p: object) => ReactNode;
let Route: (p: object) => ReactNode;
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ SaveDialog } = await vite.ssrLoadModule("/src/pages/agents/SaveMessage.tsx"));
  ({ default: SavedPage } = await vite.ssrLoadModule("/src/pages/Saved.tsx"));
  ({ MemoryRouter, Routes, Route } = await vite.ssrLoadModule("/test/withRouter.ts"));
});
after(() => vite.close());

const settle = () => act(() => new Promise((r) => setTimeout(r, 250)));

function render(node: ReactNode) {
  const div = document.body.appendChild(document.createElement("div"));
  const root = createRoot(div);
  act(() => root.render(node));
  return { div, unmount: () => act(() => root.unmount()) };
}

function type(input: HTMLInputElement, value: string) {
  const set = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    set.call(input, value);
    input.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  });
}

const click = (el: Element) => act(() => (el as HTMLElement).click());
const button = (root: Element, text: string) => [...root.querySelectorAll("button")].find((b) => b.textContent!.trim() === text)!;

test("the Save form prefills the title, takes a suggested tag and saves the message", async () => {
  answer = () => ({ body: { id: 1, title: "How to deploy", tag: "Decision" } });
  let closed = 0;
  const r = render(h(SaveDialog, { agentId: "dev", m: { id: 7, content: "## How to deploy\n\nRun start.sh" }, onClose: () => closed++ }));
  const title = document.querySelector<HTMLInputElement>('input[aria-label="Title"]')!;
  const tag = document.querySelector<HTMLInputElement>('input[aria-label="Tag"]')!;
  assert.equal(title.value, "How to deploy");
  assert.equal(tag.value, "Tutorial");
  click(button(document.body, "Decision"));
  assert.equal(tag.value, "Decision");
  await act(async () => document.querySelector("form")!.requestSubmit());
  assert.deepEqual(calls, [{ method: "POST", url: "/api/saved", body: { agent_id: "dev", message_id: 7, title: "How to deploy", tag: "Decision" } }]);
  assert.equal(closed, 1);
  r.unmount();
});

test("the Save form takes a typed tag, and keeps open with the error when saving fails", async () => {
  answer = () => ({ status: 400, body: { error: "That message isn't in this chat." } });
  let closed = 0;
  const r = render(h(SaveDialog, { agentId: "dev", m: { id: 7, content: "hello" }, onClose: () => closed++ }));
  type(document.querySelector<HTMLInputElement>('input[aria-label="Tag"]')!, "Recipes");
  await act(async () => document.querySelector("form")!.requestSubmit());
  assert.equal(calls[0].body.tag, "Recipes");
  assert.equal(closed, 0);
  assert.match(document.querySelector('[role="alert"]')!.textContent!, /isn't in this chat/);
  r.unmount();
});

const item = (id: number, title: string, tag: string) =>
  ({ id, title, tag, content: `**${title}** text`, agent_id: "dev", agent_name: "Loft Dev", message_id: id, created_at: "2026-11-23T10:00:00Z", updated_at: null });

function page(path: string) {
  return render(h(MemoryRouter, { initialEntries: [path] }, h(Routes, null, h(Route, { path: "/saved", element: h(SavedPage) }), h(Route, { path: "/saved/:id", element: h(SavedPage) }))));
}

test("the Saved page lists items and filters them by search and tag", async () => {
  answer = (url) => {
    const p = new URL(url, "http://x").searchParams;
    const all = [item(2, "Ports", "Good to know"), item(1, "Deploy", "Tutorial")];
    return { body: { items: all.filter((i) => (!p.get("tag") || i.tag === p.get("tag")) && i.title.toLowerCase().includes(p.get("q")!.toLowerCase())), tags: ["Good to know", "Recipes", "Tutorial"] } };
  };
  const r = page("/saved");
  await settle();
  const titles = () => [...r.div.querySelectorAll("li a span:first-child")].map((s) => s.textContent);
  assert.deepEqual(titles(), ["Ports", "Deploy"]);
  // The tag filter: All, the suggested tags, then the others in use.
  const tags = r.div.querySelector('[aria-label="Tag"]')!;
  assert.deepEqual([...tags.querySelectorAll("button")].map((b) => b.textContent), ["All", "Tutorial", "Good to know", "Decision", "Recipes"]);
  click(button(tags, "Tutorial"));
  await settle();
  assert.equal(calls.at(-1)!.url, "/api/saved?q=&tag=Tutorial");
  assert.deepEqual(titles(), ["Deploy"]);
  click(button(tags, "All"));
  type(r.div.querySelector<HTMLInputElement>('input[aria-label="Search saved"]')!, "port");
  await settle();
  assert.equal(calls.at(-1)!.url, "/api/saved?q=port&tag=");
  assert.deepEqual(titles(), ["Ports"]);
  assert.equal(r.div.querySelector("li a")!.getAttribute("href"), "/saved/2");
  r.unmount();
});

test("an item opens as a readable page, linking back to its message while it's there", async () => {
  answer = () => ({ body: { ...item(3, "Deploy", "Tutorial"), source_exists: true } });
  let r = page("/saved/3");
  await settle();
  assert.equal(r.div.querySelector("h1")!.textContent, "Deploy");
  assert.equal(r.div.querySelector(".prose-chat strong")!.textContent, "Deploy");
  const back = [...r.div.querySelectorAll("a")].find((a) => a.textContent!.includes("Open in chat"))!;
  assert.equal(back.getAttribute("href"), "/agents/dev#msg-3");
  r.unmount();

  answer = () => ({ body: { ...item(3, "Deploy", "Tutorial"), source_exists: false } });
  r = page("/saved/3");
  await settle();
  assert.match(r.div.textContent!, /The source message is gone/);
  assert.equal([...r.div.querySelectorAll("a")].some((a) => a.textContent!.includes("Open in chat")), false);
  r.unmount();
});
