// The Repos page in jsdom, with `api` stubbed: a failed Refresh shows its error.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/repos", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });
Object.assign(globalThis, { EventSource: class { close() {} } });

let vite: ViteDevServer;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
});
after(() => vite.close());

test("a failed Repos Refresh shows its error", async () => {
  const { createRoot } = await import("react-dom/client");
  const { api } = await vite.ssrLoadModule("/src/lib/api.ts");
  for (const k of Object.keys(api)) api[k] = () => Promise.resolve(null);
  api.github = () => Promise.resolve({});
  let fail!: () => void;
  api.repos = (refresh?: boolean) => (refresh ? new Promise((_, no) => (fail = () => no(new Error("refresh refused")))) : Promise.resolve([]));
  const { MemoryRouter } = await vite.ssrLoadModule("/test/withRouter.ts");
  const { default: ReposPage } = await vite.ssrLoadModule("/src/pages/Repos.tsx");
  const div = document.body.appendChild(document.createElement("div"));
  await act(async () => createRoot(div).render(h(MemoryRouter, null, h(ReposPage))));
  const refresh = [...div.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Refresh")!;
  assert.ok(refresh, "the Refresh button");
  await act(async () => refresh.click());
  assert.match(div.textContent ?? "", /Refreshing…/);
  await act(async () => fail());
  assert.match(div.textContent ?? "", /refresh refused/);
});
