// Renders the subagent bar in jsdom. Vite loads the component, so the .tsx compiles as it does in the app.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event"]) {
  Object.assign(globalThis, { [k]: (dom.window as any)[k] });
}

let vite: ViteDevServer;
let SubagentBar: (p: { agent: object }) => unknown;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ SubagentBar } = await vite.ssrLoadModule("/src/pages/agents/SubagentBar.tsx"));
});
after(() => vite.close());

const sub = (id: string) => ({ id, description: `Job ${id}`, type: "Explore", started_at: "2026-10-28T10:00:00Z" });

function text(agent: object) {
  const host = document.body.appendChild(document.createElement("div"));
  const root = createRoot(host);
  act(() => root.render(createElement(SubagentBar as any, { agent })));
  const t = host.textContent;
  act(() => root.unmount());
  return t;
}

test("hidden with no subagents running", () => {
  assert.equal(text({ max_subagents: 5, subagents: [] }), "");
  assert.equal(text({ max_subagents: null }), "");
});

test("counts the running subagents against the agent's limit", () => {
  assert.equal(text({ max_subagents: 5, subagents: [sub("a"), sub("b")] }), "2 of 5 subagents");
});

test("Default shows as 20", () => {
  assert.equal(text({ max_subagents: null, subagents: [sub("a")] }), "1 of 20 subagents");
});
