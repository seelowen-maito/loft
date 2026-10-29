// Runs the composer's Draft hook and the image tile in jsdom. Vite loads them, so they compile as they do in the app.
import { after, before, beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { EMPTY_DRAFT, loadDraft, saveDraft } from "../src/lib/draft.ts";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
Object.defineProperty(globalThis, "localStorage", { value: dom.window.localStorage, configurable: true });
for (const k of ["HTMLElement", "Node", "Event"]) {
  Object.assign(globalThis, { [k]: (dom.window as any)[k] });
}

type Hook = {
  text: string;
  setText: (t: string) => void;
  attachments: { key: string; file?: string }[];
  taskId: string | null;
  putDraft: (id: string, d: typeof EMPTY_DRAFT) => void;
  removeImage: (key: string) => void;
};
let vite: ViteDevServer;
let useDraft: (agent: string) => Hook;
let AttachmentTile: (p: object) => unknown;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ useDraft } = await vite.ssrLoadModule("/src/lib/useDraft.ts"));
  ({ AttachmentTile } = await vite.ssrLoadModule("/src/components/AttachmentTile.tsx"));
});
after(() => vite.close());
beforeEach(() => localStorage.clear());

/** The composer for `agent`; `show` switches it to another agent the way selecting one in the list does. */
function composer(agent: string) {
  let hook!: Hook;
  function Harness({ agent }: { agent: string }) {
    hook = useDraft(agent);
    return null;
  }
  const root: Root = createRoot(document.body.appendChild(document.createElement("div")));
  const show = (id: string) => act(() => root.render(createElement(Harness, { agent: id })));
  show(agent);
  return { now: () => hook, show, unmount: () => act(() => root.unmount()) };
}

test("switching agents shows the new agent's Draft and keeps the old one stored", () => {
  saveDraft("a", { text: "for A", images: ["a.png"], task: "t1" });
  saveDraft("b", { text: "for B", images: [], task: null });
  const c = composer("a");
  assert.equal(c.now().text, "for A");
  assert.deepEqual(c.now().attachments.map((x) => x.file), ["a.png"]);
  assert.equal(c.now().taskId, "t1");
  act(() => c.now().setText("for A, edited"));
  c.show("b");
  assert.equal(c.now().text, "for B");
  assert.deepEqual(c.now().attachments, []);
  assert.equal(c.now().taskId, null);
  assert.deepEqual(loadDraft("a"), { text: "for A, edited", images: ["a.png"], task: "t1" });
  assert.equal(loadDraft("b").text, "for B");
  c.show("a");
  assert.equal(c.now().text, "for A, edited");
  c.unmount();
});

test("a command for A that finishes while B is shown empties A's Draft and leaves B alone", () => {
  const c = composer("a");
  act(() => c.now().setText("/clear"));
  const putForA = c.now().putDraft;
  c.show("b");
  act(() => c.now().setText("B typing"));
  act(() => putForA("a", EMPTY_DRAFT));
  assert.equal(c.now().text, "B typing");
  assert.equal(loadDraft("b").text, "B typing");
  assert.deepEqual(loadDraft("a"), EMPTY_DRAFT);
  c.unmount();
});

test("a command for the shown agent empties its composer and its Draft", () => {
  saveDraft("a", { text: "hello", images: ["a.png"], task: "t1" });
  const c = composer("a");
  act(() => c.now().putDraft("a", EMPTY_DRAFT));
  assert.equal(c.now().text, "");
  assert.deepEqual(c.now().attachments, []);
  assert.equal(c.now().taskId, null);
  assert.equal(localStorage.getItem("loft:draft:a"), null);
  c.unmount();
});

test("a Draft's image whose file is gone is dropped from the composer and the Draft", () => {
  saveDraft("a", { text: "look", images: ["gone.png", "kept.png"], task: null });
  const c = composer("a");
  const host = document.body.appendChild(document.createElement("div"));
  const root = createRoot(host);
  const tile = (a: object) => act(() => root.render(createElement(AttachmentTile as any, { a, onRemove: () => {}, onMissing: () => c.now().removeImage("gone.png") })));
  // Only an uploaded image counts as missing; one still uploading has its own error path.
  tile({ key: "x", preview: "blob:x" });
  act(() => void host.querySelector("img")!.dispatchEvent(new dom.window.Event("error")));
  assert.equal(c.now().attachments.length, 2);
  tile(c.now().attachments[0]);
  act(() => void host.querySelector("img")!.dispatchEvent(new dom.window.Event("error")));
  assert.deepEqual(c.now().attachments.map((x) => x.file), ["kept.png"]);
  assert.deepEqual(loadDraft("a").images, ["kept.png"]);
  act(() => root.unmount());
  c.unmount();
});
