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

test("links in an item open as in the chat: external ones in a new tab", async () => {
  answer = () => ({ body: { ...item(4, "Docs", "Tutorial"), content: "See [docs](https://example.com)", source_exists: true } });
  const r = page("/saved/4");
  await settle();
  const a = [...r.div.querySelectorAll(".prose-chat a")].find((x) => x.textContent === "docs")!;
  assert.equal(a.getAttribute("href"), "https://example.com");
  assert.equal(a.getAttribute("target"), "_blank");
  assert.equal(a.getAttribute("rel"), "noopener noreferrer");
  r.unmount();
});

test("a missing item or a bad id says the item doesn't exist", async () => {
  answer = () => ({ status: 404, body: { error: "not found" } });
  let r = page("/saved/999");
  await settle();
  assert.match(r.div.textContent!, /This saved item doesn't exist\./);
  r.unmount();
  calls = [];
  r = page("/saved/abc");
  await settle();
  assert.match(r.div.textContent!, /This saved item doesn't exist\./);
  assert.equal(calls.length, 0);
  r.unmount();
});

function typeArea(area: HTMLTextAreaElement, value: string) {
  const set = Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype, "value")!.set!;
  act(() => {
    set.call(area, value);
    area.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
  });
}

test("Edit changes the title, tag and content, previews the Markdown as the page reads, and shows the edit time", async () => {
  answer = (url) => ({ body: url === "/api/saved/5" && calls.at(-1)!.method === "PUT"
    ? { ...item(5, "Deploy v2", "Decision"), content: "# Steps\n\n**Run** it", updated_at: "2026-11-23T12:30:00Z" }
    : { ...item(5, "Deploy", "Tutorial"), source_exists: true } });
  const r = page("/saved/5");
  await settle();
  assert.doesNotMatch(r.div.textContent!, /Edited/);
  click(button(r.div, "Edit"));
  const form = r.div.querySelector("form")!;
  type(form.querySelector<HTMLInputElement>('input[aria-label="Title"]')!, "Deploy v2");
  click(button(form, "Decision"));
  typeArea(form.querySelector<HTMLTextAreaElement>('textarea[aria-label="Content"]')!, "# Steps\n\n**Run** it");
  // The preview renders like the page: the same Markdown in the same frame.
  click(button(form, "Preview"));
  assert.equal(form.querySelector("textarea"), null);
  assert.equal(form.querySelector(".prose-chat h1")!.textContent, "Steps");
  assert.equal(form.querySelector(".prose-chat strong")!.textContent, "Run");
  click(button(form, "Write"));
  await act(async () => form.requestSubmit());
  assert.deepEqual(calls.at(-1), { method: "PUT", url: "/api/saved/5", body: { title: "Deploy v2", tag: "Decision", content: "# Steps\n\n**Run** it" } });
  await settle();
  assert.equal(r.div.querySelector("form"), null);
  assert.equal(r.div.querySelector("h1")!.textContent, "Deploy v2");
  assert.equal(r.div.querySelector(".prose-chat h1")!.textContent, "Steps");
  assert.match(r.div.textContent!, /Edited Nov 23, 2026/);
  // The source link stays after an edit.
  assert.ok([...r.div.querySelectorAll("a")].some((a) => a.textContent!.includes("Open in chat")));
  r.unmount();
});

test("Edit keeps the form open with the error when saving fails, and Cancel leaves the item as it was", async () => {
  answer = (url) => (calls.at(-1)!.method === "PUT" ? { status: 400, body: { error: "A saved item needs a title." } } : { body: { ...item(6, "Ports", "Good to know"), source_exists: true } });
  const r = page("/saved/6");
  await settle();
  click(button(r.div, "Edit"));
  type(r.div.querySelector<HTMLInputElement>('input[aria-label="Title"]')!, "Ports!");
  await act(async () => r.div.querySelector("form")!.requestSubmit());
  assert.match(r.div.querySelector('[role="alert"]')!.textContent!, /needs a title/);
  click(button(r.div, "Cancel"));
  assert.equal(r.div.querySelector("h1")!.textContent, "Ports");
  r.unmount();
});

test("Delete asks first, then removes the item and goes back to the list", async () => {
  answer = (url) => (url.startsWith("/api/saved?") ? { body: { items: [], tags: [] } } : calls.at(-1)!.method === "DELETE" ? { body: { ok: true } } : { body: { ...item(7, "Old", "Tutorial"), source_exists: true } });
  const r = page("/saved/7");
  await settle();
  click(button(r.div, "Delete"));
  // Nothing is deleted until the confirm.
  assert.equal(calls.some((c) => c.method === "DELETE"), false);
  assert.match(document.body.textContent!, /Delete this saved item\?/);
  click(button(document.body, "Keep it"));
  assert.equal(calls.some((c) => c.method === "DELETE"), false);
  click(button(r.div, "Delete"));
  const dialog = document.querySelector('[role="dialog"]')!;
  await act(async () => (button(dialog, "Delete") as HTMLElement).click());
  await settle();
  assert.deepEqual(calls.find((c) => c.method === "DELETE"), { method: "DELETE", url: "/api/saved/7", body: null });
  assert.match(r.div.textContent!, /Nothing saved yet/);
  r.unmount();
});

const pr = (number: number) => ({
  id: `pr-${number}`, agent_id: "dev", slug: "octo/site", number, url: `https://github.com/octo/site/pull/${number}`, tool_use_id: null, title: "Docs: Deploy",
  state: "open", draft: false, mergeable: true, mergeable_state: "clean", merge_methods: ["squash"], head_ref: "docs/saved-8-1", base_ref: "main", author: "octo",
  additions: 3, deletions: 0, changed_files: 1, commits: 1, comments: 0, checks: null, merged_by: null, merged_at: null, closed_at: null, error: null,
  checked_at: new Date().toISOString(), created_at: "2026-11-23T10:00:00Z",
});

test("Export to docs picks a project, opens the PR and shows its card on the item", async () => {
  answer = (url) => {
    if (url === "/api/repos") return { body: [{ name: "notes", github: null }, { name: "site", github: "octo/site" }] };
    if (url === "/api/saved/8/export") return { body: pr(12) };
    return { body: { ...item(8, "Deploy", "Tutorial"), source_exists: true, pr: null } };
  };
  const r = page("/saved/8");
  await settle();
  assert.equal(r.div.querySelector('a[href^="https://github.com"]'), null);
  click(button(r.div, "Export to docs"));
  await settle();
  const dialog = document.querySelector('[role="dialog"]')!;
  const select = dialog.querySelector<HTMLSelectElement>('select[aria-label="Project"]')!;
  // The first project on GitHub is picked; one without says why it can't be.
  assert.equal(select.value, "site");
  assert.deepEqual([...select.options].map((o) => o.textContent), ["notes (no GitHub repo)", "site (octo/site)"]);
  act(() => {
    select.value = "notes";
    select.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
  });
  assert.match(dialog.querySelector('[role="note"]')!.textContent!, /notes has no GitHub repo/);
  assert.equal(button(dialog, "Open PR").disabled, true);
  act(() => {
    select.value = "site";
    select.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
  });
  assert.equal(dialog.querySelector('[role="note"]'), null);
  await act(async () => dialog.querySelector("form")!.requestSubmit());
  assert.deepEqual(calls.find((c) => c.method === "POST"), { method: "POST", url: "/api/saved/8/export", body: { project: "site" } });
  await settle();
  assert.equal(document.querySelector('[role="dialog"]'), null);
  assert.match(r.div.textContent!, /Exported to docs: octo\/site#12/);
  assert.equal(r.div.querySelector('a[href="https://github.com/octo/site/pull/12"]')!.getAttribute("target"), "_blank");
  r.unmount();
});

test("an exported item shows its PR when opened, and an export error stays in the dialog", async () => {
  answer = (url) => {
    if (url === "/api/repos") return { body: [{ name: "site", github: "octo/site" }] };
    if (url === "/api/saved/9/export") return { status: 400, body: { error: "Couldn't export to octo/site: GitHub 403: Resource not accessible" } };
    return { body: { ...item(9, "Deploy", "Tutorial"), source_exists: true, pr: pr(11) } };
  };
  const r = page("/saved/9");
  await settle();
  assert.match(r.div.textContent!, /Exported to docs: octo\/site#11/);
  click(button(r.div, "Export to docs"));
  await settle();
  const dialog = document.querySelector('[role="dialog"]')!;
  await act(async () => dialog.querySelector("form")!.requestSubmit());
  await settle();
  assert.match(dialog.querySelector('[role="alert"]')!.textContent!, /Resource not accessible/);
  r.unmount();
});
