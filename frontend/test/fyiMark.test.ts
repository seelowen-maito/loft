// Renders the FYI mark in jsdom. Vite loads the component, so the .tsx compiles as it does in the app.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "KeyboardEvent", "FocusEvent", "MouseEvent"]) {
  Object.assign(globalThis, { [k]: (dom.window as any)[k] });
}

let vite: ViteDevServer;
let FyiMark: (p: { m: object; agentName: string }) => unknown;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ FyiMark } = await vite.ssrLoadModule("/src/pages/agents/FyiMark.tsx"));
});
after(() => vite.close());

const lead = { id: "l", name: "Lead" };
const msg = (meta: object | null, role = "user") => ({ id: 1, agent_id: "d", run_id: null, role, kind: "text", content: "Merged #12", meta, created_at: "" });
const NOTE = "FYI: delivered without waking Dev. It reads this at the start of its next run.";

function render(m: object) {
  const host = document.body.appendChild(document.createElement("div"));
  const root: Root = createRoot(host);
  act(() => root.render(createElement(FyiMark as any, { m, agentName: "Dev" })));
  return { host, root };
}

const fire = (el: Element, e: Event) => act(() => void el.dispatchEvent(e));
// jsdom has no PointerEvent: a MouseEvent carrying a pointerType is what React reads.
const pointer = (type: string, pointerType: string) => Object.assign(new dom.window.MouseEvent(type, { bubbles: true }), { pointerType });
const key = (k: string) => new dom.window.KeyboardEvent("keydown", { bubbles: true, key: k });

function parts(host: Element) {
  const button = host.querySelector("button")!;
  const note = document.getElementById(button.getAttribute("aria-describedby")!)!;
  return { button, note, open: () => button.getAttribute("aria-expanded") === "true" && note.getAttribute("class") !== "sr-only" };
}

test("only FYI messages from agents get the mark", () => {
  for (const m of [msg({ from_agent: lead }), msg({ fyi: true }), msg(null), msg({ from_agent: lead, fyi: true }, "assistant")]) {
    const { host, root } = render(m);
    assert.equal(host.innerHTML, "", JSON.stringify(m));
    act(() => root.unmount());
  }
  const { host, root } = render(msg({ from_agent: lead, fyi: true }));
  assert.equal(host.querySelector("button")?.textContent, "FYI");
  act(() => root.unmount());
});

test("the note is always there for screen readers, shown only when open", () => {
  const { host, root } = render(msg({ from_agent: lead, fyi: true }));
  const { note, open } = parts(host);
  assert.equal(note.getAttribute("role"), "tooltip");
  assert.equal(note.textContent, NOTE);
  assert.equal(note.getAttribute("class"), "sr-only");
  assert.equal(open(), false);
  act(() => root.unmount());
});

test("keyboard: Enter or Space opens it, Escape and Tab-away close it", () => {
  const { host, root } = render(msg({ from_agent: lead, fyi: true }));
  const { button, open } = parts(host);
  const other = document.body.appendChild(document.createElement("button"));
  for (const k of ["Enter", " "]) {
    act(() => button.focus());
    fire(button, key(k));
    act(() => button.click()); // what a browser does for Enter/Space on a button
    assert.equal(open(), true, `${k} opens`);
    act(() => other.focus());
    assert.equal(open(), false, `Tab-away after ${k} closes`);
  }
  act(() => button.focus());
  fire(button, key("Enter"));
  act(() => button.click());
  fire(button, key("Escape"));
  assert.equal(open(), false, "Escape closes");
  other.remove();
  act(() => root.unmount());
});

test("hover opens and leaving closes; a tap toggles and a tap elsewhere closes", () => {
  const { host, root } = render(msg({ from_agent: lead, fyi: true }));
  const { button, open } = parts(host);
  fire(button, pointer("pointerover", "mouse"));
  assert.equal(open(), true, "hover");
  fire(button, pointer("pointerout", "mouse"));
  assert.equal(open(), false, "pointer-leave");

  const tap = () => {
    fire(button, pointer("pointerdown", "touch"));
    act(() => button.click());
  };
  tap();
  assert.equal(open(), true, "tap opens");
  tap();
  assert.equal(open(), false, "second tap closes");
  tap();
  fire(document.body, pointer("pointerdown", "touch"));
  assert.equal(open(), false, "tap elsewhere closes");
  act(() => root.unmount());
});
