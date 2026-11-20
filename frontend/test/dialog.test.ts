// The shared Dialog and ConfirmDialog in jsdom: Esc closes the top one only, Tab stays inside, focus comes back.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "KeyboardEvent"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

type DialogProps = { title: string; onClose?: () => void; children?: ReactNode };
let vite: ViteDevServer;
let Dialog: (p: DialogProps) => ReactNode;
let ConfirmDialog: (p: object) => ReactNode;
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  // After jsdom is set up: react-dom checks the DOM's events once, as it loads.
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ Dialog, ConfirmDialog } = await vite.ssrLoadModule("/src/components/Dialog.tsx"));
});
after(() => vite.close());

function mount(el: ReactNode) {
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  act(() => root.render(el));
  return { render: (e: ReactNode) => act(() => root.render(e)), unmount: () => act(() => root.unmount()) };
}
const press = (key: string, o: KeyboardEventInit = {}, on: Element = document.activeElement ?? document.body) =>
  act(() => void on.dispatchEvent(new dom.window.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...o })));
const dialogs = () => [...document.querySelectorAll('[role="dialog"]')].map((d) => d.getAttribute("aria-label"));

test("a dialog is labelled, takes focus, and gives it back when it closes", () => {
  const opener = document.body.appendChild(document.createElement("button"));
  opener.focus();
  const m = mount(h(Dialog, { title: "Edit", onClose: () => {} }, h("button", null, "OK")));
  const d = document.querySelector('[role="dialog"]')!;
  assert.equal(d.getAttribute("aria-label"), "Edit");
  assert.equal(d.getAttribute("aria-modal"), "true");
  assert.ok(d.contains(document.activeElement));
  m.unmount();
  assert.equal(document.activeElement, opener);
  opener.remove();
});

test("focus goes back to the opener when the dialog autofocuses a field, also from a dialog over another", () => {
  const opener = document.body.appendChild(document.createElement("button"));
  opener.focus();
  const m = mount(h(Dialog, { title: "New", onClose: () => {} }, h("input", { autoFocus: true })));
  assert.equal(document.activeElement?.tagName, "INPUT");
  m.unmount();
  assert.equal(document.activeElement, opener);
  opener.remove();

  // The Edit dialog over the issue page: closing it leaves focus on Edit, inside the issue page.
  const page = (edit: boolean) =>
    h(Dialog, { title: "Issue", onClose: () => {} }, h("button", { id: "edit" }, "Edit"), edit && h(Dialog, { title: "Edit", onClose: () => {} }, h("input", { autoFocus: true })));
  const p = mount(page(false));
  document.getElementById("edit")!.focus();
  p.render(page(true));
  assert.equal(document.activeElement?.tagName, "INPUT");
  p.render(page(false));
  assert.equal(document.activeElement?.id, "edit");
  p.unmount();
});

test("Esc closes only the top dialog, then the one under it", () => {
  const closed: string[] = [];
  const view = (inner: boolean) =>
    h(Dialog, { title: "Outer", onClose: () => closed.push("outer") }, h("button", null, "a"), inner && h(Dialog, { title: "Inner", onClose: () => closed.push("inner") }, h("button", null, "b")));
  const m = mount(view(true));
  assert.deepEqual(dialogs(), ["Outer", "Inner"]);
  press("Escape");
  assert.deepEqual(closed, ["inner"]);
  m.render(view(false));
  press("Escape");
  assert.deepEqual(closed, ["inner", "outer"]);
  m.unmount();
});

test("Esc in an autofocused field closes the dialog", () => {
  let closed = 0;
  const m = mount(h(Dialog, { title: "New agent", onClose: () => closed++ }, h("input", { autoFocus: true, "aria-label": "Name" })));
  assert.equal(document.activeElement?.tagName, "INPUT");
  press("Escape");
  assert.equal(closed, 1);
  m.unmount();
});

test("Esc doesn't close while composing in an IME, nor when a field's menu or something else handled it", () => {
  let closed = 0;
  // A field whose menu takes Esc, as TagMenu and the @-mention popup do.
  const menu = h("input", { "aria-label": "Title", onKeyDown: (e: KeyboardEvent) => e.key === "Escape" && e.preventDefault() });
  const m = mount(h(Dialog, { title: "New", onClose: () => closed++ }, menu, h("button", null, "Save")));
  const input = document.querySelector("input")!;
  input.focus();
  press("Escape", {}, input);
  press("Escape", { isComposing: true }, input);
  document.querySelector<HTMLElement>("button")!.focus();
  press("Escape", { isComposing: true });
  const handled = new dom.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
  handled.preventDefault();
  act(() => void document.activeElement!.dispatchEvent(handled));
  assert.equal(closed, 0);
  press("Escape");
  assert.equal(closed, 1);
  m.unmount();
});

test("Tab and Shift+Tab wrap around inside the dialog", () => {
  const m = mount(h(Dialog, { title: "Run", onClose: () => {} }, h("button", null, "first"), h("button", null, "last")));
  const [first, last] = [...document.querySelectorAll<HTMLElement>('[role="dialog"] button')];
  last.focus();
  press("Tab");
  assert.equal(document.activeElement, first);
  press("Tab", { shiftKey: true });
  assert.equal(document.activeElement, last);
  m.unmount();
});

test("a destructive confirm is red; a plain one isn't, and pending disables it", () => {
  let confirmed = 0;
  const props = { title: "Delete this issue?", body: "Gone for good.", confirmLabel: "Delete", onConfirm: () => confirmed++, onClose: () => {} };
  const m = mount(h(ConfirmDialog, { ...props, danger: true }));
  const button = () => [...document.querySelectorAll("button")].find((b) => b.textContent === "Delete")!;
  assert.match(button().className, /bg-bad/);
  act(() => button().click());
  assert.equal(confirmed, 1);
  m.render(h(ConfirmDialog, { ...props, pending: true }));
  assert.doesNotMatch(button().className, /bg-bad/);
  assert.equal(button().disabled, true);
  m.unmount();
});

test("a confirm that is running shows a spinner and its working label; one that can't run yet is only disabled; its error is announced", () => {
  const props = { title: "Delete this issue?", body: "Gone.", confirmLabel: "Delete", pendingLabel: "Deleting…", danger: true, onConfirm: () => {}, onClose: () => {} };
  const m = mount(h(ConfirmDialog, { ...props, pending: true, error: "No network" }));
  const confirm = () => document.querySelector<HTMLButtonElement>('[role="dialog"] button[aria-busy]')!;
  assert.equal(confirm().textContent, "Deleting…");
  assert.equal(confirm().getAttribute("aria-busy"), "true");
  assert.ok(confirm().querySelector(".animate-spin"));
  assert.equal(document.querySelector('[role="dialog"] [role="alert"]')?.textContent, "No network");
  m.render(h(ConfirmDialog, { ...props, disabled: true }));
  assert.equal(confirm().textContent, "Delete");
  assert.equal(confirm().disabled, true);
  assert.equal(confirm().getAttribute("aria-busy"), "false");
  m.unmount();
});
