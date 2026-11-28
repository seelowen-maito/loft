// Callout, ResultLine and toast() in jsdom: one look per tone, and toasts from anywhere that go away on their own.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

let vite: ViteDevServer;
let Callout: (p: object) => ReactNode;
let ResultLine: (p: object) => ReactNode;
let Toaster: () => ReactNode;
let toast: (text: string) => void;
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ Callout, ResultLine } = await vite.ssrLoadModule("/src/components/Callout.tsx"));
  ({ Toaster, toast } = await vite.ssrLoadModule("/src/components/FloatingBar.tsx"));
});
after(() => vite.close());

function render(el: ReactNode) {
  const div = document.body.appendChild(document.createElement("div"));
  const root = createRoot(div);
  act(() => root.render(el));
  return { div, unmount: () => act(() => root.unmount()) };
}

test("a callout's tone sets its tint and whether it is announced", () => {
  const bad = render(h(Callout, { tone: "bad" }, "Couldn't save")).div.firstElementChild!;
  const warn = render(h(Callout, { tone: "warn" }, "Careful")).div.firstElementChild!;
  const note = render(h(Callout, { tone: "accent" }, "FYI")).div.firstElementChild!;
  assert.equal(bad.getAttribute("role"), "alert");
  assert.match(bad.className, /border-bad\/30 bg-bad\/\[0\.06\] text-bad/);
  assert.equal(warn.getAttribute("role"), "alert");
  assert.match(warn.className, /border-warn\/30 bg-warn\/\[0\.06\]/);
  assert.equal(note.getAttribute("role"), "status");
  assert.equal(bad.textContent, "Couldn't save");
});

test("a result line is green with a tick when it worked, red when it didn't, and nothing without a result", () => {
  const ok = render(h(ResultLine, { said: { ok: true, text: "Saved." } })).div;
  const no = render(h(ResultLine, { said: { ok: false, text: "No network" } })).div;
  assert.equal(ok.textContent, "✓ Saved.");
  assert.match(ok.firstElementChild!.className, /text-good/);
  assert.equal(no.textContent, "No network");
  assert.equal(no.firstElementChild!.getAttribute("role"), "alert");
  assert.equal(render(h(ResultLine, { said: null })).div.innerHTML, "");
});

test("toast() shows its text on the Toaster, then it goes away", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { div, unmount } = render(h(Toaster));
  assert.equal(div.textContent, "");
  act(() => toast("Nothing needs you"));
  assert.equal(div.querySelector('[role="status"]')?.textContent, "Nothing needs you");
  act(() => t.mock.timers.tick(2500));
  assert.equal(div.textContent, "");
  unmount();
  // Without a Toaster on the page, it's a no-op.
  toast("nobody listens");
});

test("success lines come from ResultLine, not a hand-made \"✓ {…}\"", async () => {
  const { readdir, readFile } = await import("node:fs/promises");
  const files = (await readdir("src", { recursive: true })).filter((f) => f.endsWith(".tsx") && !f.endsWith("Callout.tsx"));
  const handMade = [];
  for (const f of files) if ((await readFile(`src/${f}`, "utf8")).includes("✓ {")) handMade.push(f);
  assert.deepEqual(handMade, []);
});
