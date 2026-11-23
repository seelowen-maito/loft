// The "Lead and Deliveries" settings card in jsdom, with `api` stubbed: defaults, saving a change, a refused value, the log.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/settings", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });
Object.assign(globalThis, { EventSource: class { close() {} } });

let vite: ViteDevServer;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
});
after(() => vite.close());

const view = (over: object = {}, log: object[] = []) => ({
  project: "loft",
  options: { enabled: false, resumes_per_hour: 2, resumes_per_day: 6, running_deliveries: 2, ...over },
  ceilings: { resumes_per_hour: 6, resumes_per_day: 24, running_deliveries: 5 },
  log,
});

test("the Lead option starts off, saves a change and shows a refusal and the log", async () => {
  const { createRoot } = await import("react-dom/client");
  const { api } = await vite.ssrLoadModule("/src/lib/api.ts");
  const { LeadDeliveryOption, changeText } = await vite.ssrLoadModule("/src/pages/LeadDeliveryOption.tsx");
  api.repos = () => Promise.resolve([{ name: "loft" }]);
  api.leadDelivery = () => Promise.resolve(view());
  const sent: unknown[] = [];
  api.saveLeadDelivery = (project: string, change: any) => {
    sent.push([project, change]);
    if (change.resumes_per_hour === 99) return Promise.reject(new Error("resumes_per_hour must be a whole number from 1 to 6"));
    const at = "2026-11-23T10:00:00Z";
    return Promise.resolve(view(change, [{ at, by: "user", before: view().options, after: view(change).options }]));
  };
  const div = document.body.appendChild(document.createElement("div"));
  await act(async () => createRoot(div).render(h(LeadDeliveryOption)));
  await act(async () => {});
  const sw = div.querySelector("[role=switch]")!;
  assert.equal(sw.getAttribute("aria-checked"), "false");
  assert.match(div.textContent!, /No changes yet/);
  await act(async () => (sw as HTMLElement).click());
  assert.deepEqual(sent[0], ["loft", { enabled: true }]);
  assert.match(div.textContent!, /user: turned on/);
  assert.equal(div.querySelectorAll("input[type=number]").length, 3);
  assert.equal((div.querySelector("input[type=number]") as HTMLInputElement).max, "6");
  assert.equal(changeText(view().options, view({ resumes_per_day: 10 }).options), "resumes per day: 6 → 10");
});
