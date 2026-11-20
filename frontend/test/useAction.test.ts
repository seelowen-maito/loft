// useAction, errorText, ActionButton and ErrorLine in jsdom: busy while it runs, one error text, one look while working.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

type Action = { busy: boolean; error: string | null; run: <T>(fn: () => Promise<T>) => Promise<T | undefined> };
let vite: ViteDevServer;
let useAction: () => Action;
let errorText: (e: unknown) => string;
let ActionButton: (p: object) => ReactNode;
let ErrorLine: (p: object) => ReactNode;
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ useAction, errorText } = await vite.ssrLoadModule("/src/lib/useAction.ts"));
  ({ ActionButton } = await vite.ssrLoadModule("/src/components/ActionButton.tsx"));
  ({ ErrorLine } = await vite.ssrLoadModule("/src/components/Callout.tsx"));
});
after(() => vite.close());

function render(el: ReactNode) {
  const div = document.body.appendChild(document.createElement("div"));
  const root = createRoot(div);
  act(() => root.render(el));
  return { div, render: (e: ReactNode) => act(() => root.render(e)), unmount: () => act(() => root.unmount()) };
}
function hook() {
  let a!: Action;
  const Harness = () => ((a = useAction()), null);
  render(h(Harness));
  return () => a;
}
function deferred<T>() {
  let resolve!: (v: T) => void, reject!: (e: unknown) => void;
  const promise = new Promise<T>((y, n) => ((resolve = y), (reject = n)));
  return { promise, resolve, reject };
}

test("errorText reads an Error's message, or the thing itself", () => {
  assert.equal(errorText(new Error("No network")), "No network");
  assert.equal(errorText("timed out"), "timed out");
});

test("useAction is busy while it runs and returns what it got", async () => {
  const a = hook();
  const d = deferred<number>();
  let got: Promise<number | undefined>;
  act(() => void (got = a().run(() => d.promise)));
  assert.equal(a().busy, true);
  await act(async () => d.resolve(7));
  assert.equal(await got!, 7);
  assert.equal(a().busy, false);
  assert.equal(a().error, null);
});

test("useAction keeps the error's text, and clears it on the next run", async () => {
  const a = hook();
  let got: unknown = "unset";
  await act(async () => void (got = await a().run(() => Promise.reject(new Error("Agent is busy")))));
  assert.equal(got, undefined);
  assert.equal(a().error, "Agent is busy");
  assert.equal(a().busy, false);
  await act(async () => void (await a().run(() => Promise.resolve(1))));
  assert.equal(a().error, null);
});

test("useAction ignores a second run while the first is going", async () => {
  const a = hook();
  const d = deferred<void>();
  let calls = 0;
  act(() => void a().run(() => (calls++, d.promise)));
  act(() => void a().run(() => (calls++, d.promise)));
  await act(async () => d.resolve());
  assert.equal(calls, 1);
});

test("an action button shows a spinner and its working label while pending, at full strength", () => {
  const r = render(h(ActionButton, { icon: h("svg", { id: "idle" }), pendingLabel: "Stopping…" }, "Stop"));
  const b = () => r.div.querySelector("button")!;
  assert.equal(b().textContent, "Stop");
  assert.ok(b().querySelector("#idle"));
  assert.equal(b().disabled, false);
  r.render(h(ActionButton, { icon: h("svg", { id: "idle" }), pending: true, pendingLabel: "Stopping…" }, "Stop"));
  assert.equal(b().textContent, "Stopping…");
  assert.equal(b().getAttribute("aria-busy"), "true");
  assert.equal(b().disabled, true);
  assert.ok(b().querySelector(".animate-spin"));
  assert.ok(!b().querySelector("#idle"));
  assert.match(b().className, /disabled:opacity-100/);
  r.render(h(ActionButton, { variant: "danger", pending: true }, "Delete"));
  assert.equal(b().textContent, "Delete");
  assert.match(b().className, /bg-bad\/20/);
});

test("an error line is red and announced, and empty without an error", () => {
  const r = render(h(ErrorLine, { error: "No network" }));
  assert.equal(r.div.firstElementChild!.getAttribute("role"), "alert");
  assert.equal(r.div.textContent, "No network");
  assert.equal(render(h(ErrorLine, { error: null })).div.innerHTML, "");
});

test("Start anyway on the waits-on warning shows the start working, like Start delivery", async () => {
  const { StartWarning } = await vite.ssrLoadModule("/src/components/StartWarning.tsx");
  const waits = [{ blocker: "loft#12", repo: "loft", spec: 10, issue_id: "x" }];
  const props = { waits, onStart: () => {}, onCancel: () => {}, onStarted: () => {} };
  const r = render(h(StartWarning, { ...props, startPending: true }));
  const busy = r.div.querySelector('button[aria-busy="true"]')!;
  assert.equal(busy.textContent, "Starting delivery…");
  assert.ok(busy.querySelector(".animate-spin"));
  r.render(h(StartWarning, { ...props, startPending: false }));
  assert.ok([...r.div.querySelectorAll("button")].some((b) => b.textContent === "Start anyway" && !b.disabled));
});

test("no button calls the API straight from its click: it goes through useAction", async () => {
  const { readdir, readFile } = await import("node:fs/promises");
  const files = (await readdir("src", { recursive: true })).filter((f) => f.endsWith(".tsx"));
  const raw: string[] = [];
  for (const f of files) {
    const lines = (await readFile(`src/${f}`, "utf8")).split("\n");
    lines.forEach((l, i) => /on(Click|Submit|Confirm)=\{\([^)]*\)\s*=>\s*(\{\s*)?(void\s+)?api\.\w+\(/.test(l) && raw.push(`${f}:${i + 1}`));
  }
  assert.deepEqual(raw, []);
});
