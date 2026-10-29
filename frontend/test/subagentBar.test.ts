// Renders the subagent bar in jsdom. Vite loads the component, so the .tsx compiles as it does in the app.
import { after, before, mock, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "MouseEvent"]) {
  Object.assign(globalThis, { [k]: (dom.window as any)[k] });
}

let vite: ViteDevServer;
let SubagentBar: (p: { agent: object }) => unknown;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ SubagentBar } = await vite.ssrLoadModule("/src/pages/agents/SubagentBar.tsx"));
});
after(() => vite.close());

const NOW = Date.parse("2026-10-28T10:01:05Z");
const sub = (id: string, type = "Explore") => ({ id, description: `Job ${id}`, type, started_at: "2026-10-28T10:00:00Z" });

function render(agent: object) {
  const host = document.body.appendChild(document.createElement("div"));
  const root: Root = createRoot(host);
  act(() => root.render(createElement(SubagentBar as any, { agent })));
  return { host, root };
}

const lines = (host: Element) => [...host.querySelectorAll("li")].map((li) => [...li.children].map((c) => c.textContent));
const count = (host: Element) => host.querySelector("[role=status] span")?.textContent;

test("hidden with no subagents running", () => {
  for (const agent of [{ max_subagents: 5, subagents: [] }, { max_subagents: null }]) {
    const { host, root } = render(agent);
    assert.equal(host.innerHTML, "");
    act(() => root.unmount());
  }
});

test("counts the running subagents against the agent's limit, Default as 20", () => {
  for (const [max, want] of [[5, "2 of 5 subagents"], [null, "2 of 20 subagents"]] as const) {
    const { host, root } = render({ max_subagents: max, subagents: [sub("a"), sub("b")] });
    assert.equal(count(host), want);
    act(() => root.unmount());
  }
});

test("one line per subagent with description, type and a ticking running time", () => {
  mock.timers.enable({ apis: ["setInterval", "Date"], now: NOW });
  try {
    const { host, root } = render({ max_subagents: null, subagents: [sub("a"), sub("b", "general-purpose"), sub("c")] });
    assert.deepEqual(lines(host), [["Job a", "Explore", "1m 5s"], ["Job b", "general-purpose", "1m 5s"], ["Job c", "Explore", "1m 5s"]]);
    assert.equal(host.querySelector("button"), null, "up to 3 there's nothing to expand");
    act(() => mock.timers.tick(2000));
    assert.equal(lines(host)[0][2], "1m 7s");
    act(() => root.unmount());
  } finally {
    mock.timers.reset();
  }
});

test("past 3 it shows only the count until tapped, and a tap collapses it again", () => {
  const { host, root } = render({ max_subagents: 5, subagents: ["a", "b", "c", "d"].map((id) => sub(id)) });
  const button = host.querySelector("button")!;
  assert.equal(button.textContent, "4 of 5 subagents");
  assert.equal(button.getAttribute("aria-expanded"), "false");
  assert.equal(lines(host).length, 0);
  act(() => button.click());
  assert.equal(button.getAttribute("aria-expanded"), "true");
  assert.deepEqual(lines(host).map((l) => l[0]), ["Job a", "Job b", "Job c", "Job d"]);
  act(() => button.click());
  assert.equal(lines(host).length, 0);
  act(() => root.unmount());
});
