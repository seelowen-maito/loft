// The top bar's Help menu and the icon buttons' tooltips (#712), in jsdom against a fake API.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement as h, type ReactNode } from "react";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/", pretendToBeVisual: true });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
for (const k of ["HTMLElement", "Node", "Event", "MouseEvent", "KeyboardEvent", "CustomEvent", "localStorage", "matchMedia"]) Object.assign(globalThis, { [k]: (dom.window as any)[k] });

Object.assign(globalThis, { EventSource: class { close() {} } });
const COMMANDS = [{ name: "/clear", description: "Clear the context", tag: "loft", reason: null }, { name: "/loop", description: "Repeat a prompt", tag: "loft", reason: null }];
globalThis.fetch = (async (url: string) => {
  const body = url.endsWith("/agents/a1/commands") ? COMMANDS : url.endsWith("/agents") ? [{ id: "a1", name: "Dev" }] : [];
  return new Response(JSON.stringify(body), { status: 200 });
}) as typeof fetch;

let vite: ViteDevServer;
let HelpMenu: (p: { live: boolean }) => ReactNode;
let Tip: (p: object) => ReactNode;
let OPEN_SHORTCUT: string;
let MemoryRouter: (p: object) => ReactNode;
let createRoot: typeof import("react-dom/client").createRoot;
before(async () => {
  ({ createRoot } = await import("react-dom/client"));
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ HelpMenu } = await vite.ssrLoadModule("/src/components/HelpMenu.tsx"));
  ({ Tip } = await vite.ssrLoadModule("/src/components/HoverTip.tsx"));
  ({ OPEN_SHORTCUT } = await vite.ssrLoadModule("/src/lib/shortcuts.ts"));
  ({ MemoryRouter } = await vite.ssrLoadModule("/test/withRouter.ts"));
});
after(() => vite.close());

const settle = () => act(() => new Promise((r) => setTimeout(r, 100)));
function render(node: ReactNode) {
  const div = document.body.appendChild(document.createElement("div"));
  act(() => createRoot(div).render(h(MemoryRouter as any, { initialEntries: ["/agents/a1"] }, node)));
  return div;
}
const click = (el: Element) => act(() => void el.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true })));
const items = (div: Element) => [...div.querySelectorAll('[role="menuitem"]')].map((i) => i.textContent?.replace(/\s+/g, " ").trim());
const help = (div: Element) => div.querySelector('button[aria-label="Help"]')!;

test("Help opens its six items, and the phone's connection line says whether it's live", () => {
  const div = render(h(HelpMenu as any, { live: true }));
  assert.equal(div.querySelector('[role="menu"]'), null);
  click(help(div));
  assert.equal(help(div).getAttribute("aria-expanded"), "true");
  assert.deepEqual(items(div).map((t) => t!.replace(/ (T|\?|\/|.?K|terms Loft uses|↗)$/, "")), ["Talk to an agent", "Keyboard shortcuts", "Chat commands", "Search and jump", "Glossary", "Loft on GitHub"]);
  assert.match(div.querySelector('[role="menu"]')!.textContent!, /Connected/);
  assert.equal(div.querySelector<HTMLAnchorElement>('a[role="menuitem"]')!.href, "https://github.com/get-loft/loft");
  const off = render(h(HelpMenu as any, { live: false }));
  click(help(off));
  assert.match(off.querySelector('[role="menu"]')!.textContent!, /Reconnecting…/);
});

test("Talk, Keyboard shortcuts and Search and jump open the Talk dialog, the existing sheet and the switcher", () => {
  const opened: string[] = [];
  window.addEventListener(OPEN_SHORTCUT, (e) => opened.push((e as CustomEvent).detail));
  window.addEventListener("loft:talk", () => opened.push("talk"));
  const div = render(h(HelpMenu as any, { live: true }));
  const pick = (name: string) => {
    click(help(div));
    click([...div.querySelectorAll('[role="menuitem"]')].find((i) => i.textContent!.trim().startsWith(name))!);
  };
  pick("Talk to an agent");
  pick("Keyboard shortcuts");
  assert.equal(div.querySelector('[role="menu"]'), null, "picking closes the menu");
  pick("Search and jump");
  assert.deepEqual(opened, ["talk", "sheet", "switcher"]);
});

test("Chat commands lists the agent's commands, one line each; Glossary lists GLOSSARY.md's terms", async () => {
  const div = render(h(HelpMenu as any, { live: true }));
  click(help(div));
  click([...div.querySelectorAll('[role="menuitem"]')].find((i) => i.textContent!.trim().startsWith("Chat commands"))!);
  await settle();
  const rows = [...document.querySelectorAll('[role="dialog"] li')].map((l) => l.textContent);
  assert.deepEqual(rows, ["/clearClear the context", "/loopRepeat a prompt"]);
  click(document.querySelector('[role="dialog"] button[aria-label="Close"]')!);
  click(help(div));
  click([...div.querySelectorAll('[role="menuitem"]')].find((i) => i.textContent!.trim().startsWith("Glossary"))!);
  const terms = [...document.querySelectorAll('[role="dialog"] dt')].map((t) => t.textContent);
  assert.ok(terms.includes("Agent") && terms.includes("Delivery"), terms.join(", "));
});

test("a tooltip names the control and its shortcut, and the control keeps its aria-label", () => {
  const div = render(h(Tip as any, { name: "Agents", keys: "G A" }, h("a", { href: "/agents", "aria-label": "Agents" }, "icon")));
  const tip = div.querySelector('[aria-hidden="true"]')!;
  assert.equal(tip.textContent, "AgentsGA");
  assert.deepEqual([...tip.querySelectorAll("kbd")].map((k) => k.textContent), ["G", "A"]);
  assert.equal(div.querySelector("a")!.getAttribute("aria-label"), "Agents");
  assert.equal(div.querySelector("a")!.hasAttribute("title"), false);
});
