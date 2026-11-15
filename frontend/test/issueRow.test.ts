// The row shared by the issue picker and the chat's `#` menu, and the `#` menu's group titles.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement, useState } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "MouseEvent", "KeyboardEvent"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

const ISSUES = [
  { id: "a", title: "Chat menu", labels: [], state: "open", status: "in_progress", project: "loft", agent_id: "dev", updated_at: new Date().toISOString(), github: { repo: "o/loft", number: 527 } },
  { id: "s", title: "Voice spec", labels: ["spec"], state: "open", status: "ready", project: "loft", agent_id: null, updated_at: new Date().toISOString(), github: { repo: "o/loft", number: 500 } },
];
const PRS = [{ url: "u", slug: "o/loft", number: 528, title: "Guard PR", state: "open", draft: true, agent_id: "dev", created_at: new Date().toISOString(), merged_at: null, closed_at: null }];
const json = (v: unknown) => new Response(JSON.stringify(v), { headers: { "content-type": "application/json" } });
// No server here: the live events never arrive.
class NoEvents { addEventListener() {} close() {} }
Object.assign(globalThis, { EventSource: NoEvents, fetch: async (u: string) => json(String(u).startsWith("/api/issues") ? ISSUES : String(u).startsWith("/api/pull-requests") ? PRS : []) });

let vite: ViteDevServer;
let IssueRow: (p: object) => unknown;
let useTagMenu: (project: string, agents: object[], text: string, set: (f: unknown) => void) => { menu: unknown; onKeyDown: (e: object) => boolean };
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ IssueRow } = await vite.ssrLoadModule("/src/components/IssueRow.tsx"));
  ({ useTagMenu } = await vite.ssrLoadModule("/src/components/TagMenu.tsx"));
});
after(() => vite.close());

function mount(el: unknown) {
  const host = document.body.appendChild(document.createElement("div"));
  act(() => createRoot(host).render(el as never));
  return host;
}

test("a row shows the status tone, where it lives, the title and the meta line without the empty parts", () => {
  let picked = 0;
  const host = mount(createElement(IssueRow as never, {
    tone: "text-warn", github: true, title: "Fix it", meta: ["#7", "waiting approval", undefined, "loft", false, "1h ago"],
    selected: true, onHover: () => {}, onPick: () => picked++,
  }));
  const row = host.querySelector('[role="option"]')!;
  assert.equal(row.getAttribute("aria-selected"), "true");
  assert.match(row.querySelector("svg")!.getAttribute("class")!, /text-warn/);
  assert.ok(row.querySelector('[aria-label="On GitHub"]'));
  assert.equal(row.querySelectorAll("span")[0].textContent, "Fix it");
  assert.equal(row.querySelectorAll("span")[1].textContent, "#7 · waiting approval · loft · 1h ago");
  act(() => void row.dispatchEvent(new dom.window.MouseEvent("mousedown", { bubbles: true })));
  assert.equal(picked, 1);
});

test("the # menu groups its rows under Specs, Issues and Pull requests, and Enter still inserts #N", async () => {
  let api!: ReturnType<typeof useTagMenu>;
  let text = "see #";
  function Host() {
    const [t, setT] = useState("see #");
    text = t;
    api = useTagMenu("loft", [{ id: "dev", name: "Loft Dev" }], t, setT as never);
    return api.menu as never;
  }
  const host = mount(createElement(Host));
  await act(async () => { await new Promise((r) => setTimeout(r, 20)); });
  const titles = [...host.querySelectorAll('[role="listbox"] > div')].map((d) => d.textContent);
  assert.deepEqual(titles, ["Specs", "Issues", "Pull requests"]);
  const metas = [...host.querySelectorAll('[role="option"]')].map((o) => o.querySelectorAll("span")[1].textContent);
  assert.match(metas[2], /^#528 · draft · Loft Dev · loft · /);
  const key = (k: string) => ({ key: k, shiftKey: false, nativeEvent: { isComposing: false }, preventDefault() {} });
  act(() => void api.onKeyDown(key("ArrowDown")));
  act(() => void api.onKeyDown(key("Enter")));
  assert.equal(text, "see #527 ");
});
