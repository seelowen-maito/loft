// A chat message's footer in jsdom (#666): Reply, the time and the queued state in one row, the last thing in the message.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "localStorage", "matchMedia"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

let vite: ViteDevServer;
let MessageRow: (p: object) => ReactNode;
let MemoryRouter: (p: object) => ReactNode;
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ MessageRow } = await vite.ssrLoadModule("/src/pages/agents/Chat.tsx"));
  ({ MemoryRouter } = await vite.ssrLoadModule("/test/withRouter.ts"));
});
after(() => vite.close());

const lead = { id: "lead", name: "Loft Lead", avatar: { svg: "" }, role: "lead" };
const dev = { id: "dev", name: "Loft Dev", avatar: { svg: "" }, role: "dev", project: "loft", queued_messages: [5, 6] };
const msg = (id: number, role: string, meta: object | null = null) =>
  ({ id, agent_id: "dev", role, kind: "text", content: `message ${id}`, meta, created_at: "2026-11-23T14:05:00Z" });
const noop = () => {};

function row(m: object) {
  const div = document.body.appendChild(document.createElement("div"));
  const root = createRoot(div);
  act(() => root.render(h(MemoryRouter, null, h(MessageRow, { m, agent: dev, agents: [dev, lead], grill: null, onClearWith: async () => {}, onReply: noop, onCopy: noop, onJump: noop }))));
  const buttons = [...div.querySelectorAll("button")];
  const footer = div.querySelector("time")!.parentElement!;
  return { div, footer, labels: buttons.map((b) => b.getAttribute("aria-label") ?? b.textContent!.trim()), inFooter: buttons.every((b) => footer.contains(b)) };
}

test("your message ends with the time, then Reply naming you", () => {
  const r = row(msg(1, "user"));
  assert.deepEqual(r.labels, ["Reply to your message"]);
  assert.equal(r.footer.textContent!.endsWith("Reply"), true);
  assert.equal(r.footer.querySelector("time")!.getAttribute("title")!.length > 0, true);
});

test("the agent's message starts its footer with Reply, and nothing floats outside it", () => {
  const r = row(msg(2, "assistant"));
  assert.deepEqual(r.labels, ["Reply to Loft Dev"]);
  assert.equal(r.inFooter, true);
  assert.equal(r.footer.firstElementChild!.tagName, "BUTTON");
});

test("a queued message from an agent shows the mark and Cancel before the time and Reply", () => {
  const r = row(msg(5, "user", { from_agent: lead }));
  assert.deepEqual(r.labels, ["Cancel this queued message from Loft Lead", "Reply to Loft Lead"]);
  assert.equal(r.inFooter, true);
  assert.match(r.footer.textContent!, /^\s*Queued until this run ends Cancel.*Reply$/);
});

test("a cancelled message offers Copy to box and no Reply", () => {
  const r = row(msg(4, "user", { from_agent: lead, cancelled: true }));
  assert.deepEqual(r.labels, ["Copy to box"]);
  assert.equal(r.inFooter, true);
  assert.match(r.footer.textContent!, /^Cancelled, not sent to the agent/);
});

test("your own queued message shows the mark and Cancel in its footer", () => {
  const r = row(msg(6, "user"));
  assert.deepEqual(r.labels, ["Cancel this queued message", "Reply to your message"]);
  assert.equal(r.inFooter, true);
  assert.match(r.footer.textContent!, /^\s*Queued until this run ends Cancel.*Reply$/);
});

test("your own cancelled message is struck through, offers Copy to box and no Reply", () => {
  const r = row(msg(7, "user", { cancelled: true }));
  assert.deepEqual(r.labels, ["Copy to box"]);
  assert.match(r.footer.textContent!, /^Cancelled, not sent to the agent/);
  assert.ok(r.div.querySelector(".line-through"));
});
