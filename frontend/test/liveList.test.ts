// The shared live lists (agents, pull requests, repos, permissions, issues): loaded once, kept current by events,
// loaded again after a reconnect. Vite loads the store so it compiles as it does in the app; the hook runs in jsdom.
import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { createServer, type ViteDevServer } from "vite";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/" });
Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true, HTMLElement: dom.window.HTMLElement });
// The hook opens the event stream; here it never connects.
Object.assign(globalThis, { EventSource: class { onopen = null; onerror = null; onmessage = null } });

type Item = { id: string; v: number };
type Ev = { type: "put"; item: Item } | { type: "gone"; id: string } | { type: "other" };
type Store = {
  get: () => { items: Item[] | null; failed: string | null };
  ensure: () => void;
  reload: () => Promise<void>;
  event: (e: Ev) => void;
  connection: (live: boolean) => void;
  set: (fn: (items: Item[]) => Item[]) => void;
  use: () => { items: Item[] | null; failed: string | null };
};
let vite: ViteDevServer;
let createLiveList: (load: () => Promise<Item[]>, apply: (items: Item[], e: Ev) => Item[]) => Store;
before(async () => {
  vite = await createServer({ logLevel: "silent", appType: "custom", server: { middlewareMode: true, hmr: false } });
  ({ createLiveList } = await vite.ssrLoadModule("/src/hooks/liveList.ts"));
});
after(() => vite.close());

const apply = (items: Item[], e: Ev) => {
  if (e.type === "put") return items.some((i) => i.id === e.item.id) ? items.map((i) => (i.id === e.item.id ? e.item : i)) : [e.item, ...items];
  if (e.type === "gone") return items.filter((i) => i.id !== e.id);
  return items;
};

/** A list whose loads the test answers by hand: `answer(items)` resolves the oldest pending one. */
function list() {
  const pending: { ok: (v: Item[]) => void; fail: (e: Error) => void }[] = [];
  let loads = 0;
  const store = createLiveList(() => {
    loads++;
    return new Promise((ok, fail) => pending.push({ ok, fail }));
  }, apply);
  const flush = () => new Promise((r) => setTimeout(r, 0));
  return {
    store,
    loads: () => loads,
    answer: async (items: Item[]) => (pending.shift()!.ok(items), flush()),
    fail: async () => (pending.shift()!.fail(new Error("down")), flush()),
  };
}

test("it loads once however often it's asked for, and is null until then", async () => {
  const l = list();
  assert.equal(l.store.get().items, null);
  l.store.ensure();
  l.store.ensure();
  await l.answer([{ id: "a", v: 1 }]);
  l.store.ensure();
  assert.equal(l.loads(), 1);
  assert.deepEqual(l.store.get(), { items: [{ id: "a", v: 1 }], failed: null });
});

test("events update the loaded list, and others leave it as it is", async () => {
  const l = list();
  l.store.ensure();
  await l.answer([{ id: "a", v: 1 }, { id: "b", v: 1 }]);
  l.store.event({ type: "put", item: { id: "a", v: 2 } });
  l.store.event({ type: "put", item: { id: "c", v: 1 } });
  l.store.event({ type: "gone", id: "b" });
  const before = l.store.get();
  l.store.event({ type: "other" });
  assert.equal(l.store.get(), before);
  assert.deepEqual(before.items, [{ id: "c", v: 1 }, { id: "a", v: 2 }]);
});

test("events that arrive while it loads win over what loads", async () => {
  const l = list();
  l.store.ensure();
  l.store.event({ type: "put", item: { id: "a", v: 2 } });
  l.store.event({ type: "gone", id: "b" });
  await l.answer([{ id: "a", v: 1 }, { id: "b", v: 1 }]);
  assert.deepEqual(l.store.get().items, [{ id: "a", v: 2 }]);
});

test("after a reconnect it loads again; the first connect doesn't", async () => {
  const l = list();
  l.store.ensure();
  l.store.connection(true);
  await l.answer([{ id: "a", v: 1 }]);
  assert.equal(l.loads(), 1);
  l.store.connection(false);
  l.store.connection(true);
  assert.equal(l.loads(), 2);
  await l.answer([{ id: "a", v: 3 }, { id: "b", v: 1 }]);
  assert.deepEqual(l.store.get().items, [{ id: "a", v: 3 }, { id: "b", v: 1 }]);
});

test("a list nobody asked for doesn't load on a reconnect", () => {
  const l = list();
  l.store.connection(true);
  l.store.connection(false);
  l.store.connection(true);
  assert.equal(l.loads(), 0);
});

test("a failed load says so, and the next ask tries again", async () => {
  const l = list();
  l.store.ensure();
  await l.fail();
  assert.deepEqual(l.store.get(), { items: null, failed: "down" });
  l.store.ensure();
  await l.answer([{ id: "a", v: 1 }]);
  assert.deepEqual(l.store.get(), { items: [{ id: "a", v: 1 }], failed: null });
});

test("a first load that failed loads again when the stream opens", async () => {
  const l = list();
  l.store.ensure();
  await l.fail();
  l.store.connection(false);
  l.store.connection(true);
  assert.equal(l.loads(), 2, "the backend is back: the list doesn't wait for a remount");
  await l.answer([{ id: "a", v: 1 }]);
  assert.deepEqual(l.store.get(), { items: [{ id: "a", v: 1 }], failed: null });
  l.store.connection(true);
  assert.equal(l.loads(), 2, "loaded: an open stream alone doesn't load it again");
});

test("a failed reload keeps what was there", async () => {
  const l = list();
  l.store.ensure();
  await l.answer([{ id: "a", v: 1 }]);
  l.store.reload();
  await l.fail();
  assert.deepEqual(l.store.get(), { items: [{ id: "a", v: 1 }], failed: null });
});

test("an edit in place shows in every component, from one load", async () => {
  const l = list();
  const seen: (Item[] | null)[][] = [[], []];
  const Shows = ({ n }: { n: number }) => {
    seen[n].push(l.store.use().items);
    return null;
  };
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  act(() => root.render([createElement(Shows, { n: 0, key: 0 }), createElement(Shows, { n: 1, key: 1 })]));
  await act(() => l.answer([{ id: "a", v: 1 }]));
  act(() => l.store.set((items) => items.map((i) => ({ ...i, v: 9 }))));
  assert.equal(l.loads(), 1);
  for (const s of seen) assert.deepEqual(s.at(-1), [{ id: "a", v: 9 }]);
  act(() => root.unmount());
});
