// Renders the chat's video player and the composer's video tile in jsdom. Vite loads them, so they compile as they do in the app.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event"]) {
  Object.assign(globalThis, { [k]: (dom.window as any)[k] });
}

let vite: ViteDevServer;
let VideoMessage: (p: object) => unknown;
let AttachmentTile: (p: object) => unknown;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ VideoMessage } = await vite.ssrLoadModule("/src/components/VideoMessage.tsx"));
  ({ AttachmentTile } = await vite.ssrLoadModule("/src/components/AttachmentTile.tsx"));
});
after(() => vite.close());

function render(el: unknown) {
  const host = document.body.appendChild(document.createElement("div"));
  act(() => createRoot(host).render(el as any));
  return host;
}

test("a sent video plays inline and shows its length once known", () => {
  const host = render(createElement(VideoMessage as any, { src: "/api/agents/a/uploads/v.mp4" }));
  const video = host.querySelector("video")!;
  assert.equal(video.getAttribute("src"), "/api/agents/a/uploads/v.mp4");
  assert.equal(video.controls, true);
  assert.equal(video.hasAttribute("playsinline"), true, "plays in the page on iPhone, not fullscreen");
  assert.equal(host.textContent, "");
  Object.defineProperty(video, "duration", { value: 42.3 });
  act(() => video.dispatchEvent(new dom.window.Event("loadedmetadata")));
  assert.equal(host.querySelector("figcaption")?.textContent, "Video · 0:42");
});

test("a video attachment previews as a video, not an image", () => {
  const host = render(createElement(AttachmentTile as any, { a: { key: "k", preview: "blob:x", video: true }, onRemove: () => {}, onMissing: () => {} }));
  assert.equal(host.querySelector("img"), null);
  assert.equal(host.querySelector("video")?.getAttribute("src"), "blob:x");
  assert.equal(host.querySelector('[role="status"]')?.getAttribute("aria-label"), "Uploading video");
});
