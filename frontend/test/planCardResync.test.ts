// A plan card in a chat loads its plan again when the event stream lagged and the backend sent `resync`.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/" });
let stream: { onmessage: ((m: { data: string }) => void) | null } | null = null;
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  IS_REACT_ACT_ENVIRONMENT: true,
  HTMLElement: dom.window.HTMLElement,
  EventSource: class {
    onopen = null;
    onerror = null;
    onmessage: ((m: { data: string }) => void) | null = null;
    constructor() {
      stream = this;
    }
  },
});
let plans = 0;
globalThis.fetch = (async () => {
  plans++;
  return new Response(JSON.stringify({ error: "no plan" }), { status: 404 });
}) as typeof fetch;

let vite: ViteDevServer;
let PlanCard: (p: { taskId: string; agents: [] }) => unknown;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ PlanCard } = await vite.ssrLoadModule("/src/pages/agents/PlanCard.tsx"));
});
after(() => vite.close());

test("a resync event loads the plan again", async () => {
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  await act(async () => root.render(createElement(PlanCard as never, { taskId: "t1", agents: [] })));
  assert.equal(plans, 1);
  await act(async () => stream!.onmessage!({ data: JSON.stringify({ type: "resync" }) }));
  assert.equal(plans, 2);
  await act(async () => root.unmount());
});
