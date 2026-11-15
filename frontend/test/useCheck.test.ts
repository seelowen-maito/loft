import { test } from "node:test";
import assert from "node:assert/strict";
import { afterCheck, isLoading } from "../src/lib/useCheck.ts";

test("a good check replaces the data and clears an earlier error", () => {
  assert.deepEqual(afterCheck({ data: null, err: "boom" }, { ok: 1 }), { data: 1, err: null });
});

test("a failed check keeps the last data and shows its error", () => {
  assert.deepEqual(afterCheck({ data: 1, err: null }, { error: new Error("offline") }), { data: 1, err: "offline" });
});

test("a manual card (Mac access) never shows a first-check skeleton, since nothing runs on open", () => {
  assert.equal(isLoading({ data: null, err: null }, true), false);
  assert.equal(isLoading({ data: null, err: null }, false), true);
  assert.equal(isLoading({ data: 1, err: null }, false), false);
});

test("the Mac access card checks only on its button", async () => {
  const { readFileSync } = await import("node:fs");
  assert.match(readFileSync(new URL("../src/pages/MacAccess.tsx", import.meta.url), "utf8"), /useCheck\(api\.macAccess, \{ manual: true \}\)/);
});

test("a slow earlier check doesn't overwrite a newer one", async () => {
  const { JSDOM } = await import("jsdom");
  const dom = new JSDOM("<!doctype html><body></body>", { url: "http://localhost/" });
  Object.assign(globalThis, { window: dom.window, document: dom.window.document, IS_REACT_ACT_ENVIRONMENT: true });
  const { act, createElement } = await import("react");
  const { createRoot } = await import("react-dom/client");
  const { useCheck } = await import("../src/lib/useCheck.ts");

  const answers: ((v: string) => void)[] = [];
  const check = () => new Promise<string>((ok) => answers.push(ok));
  let shown: ReturnType<typeof useCheck<string>> | undefined;
  const Card = () => ((shown = useCheck(check)), null);
  const root = createRoot(document.body.appendChild(document.createElement("div")));
  await act(async () => root.render(createElement(Card)));
  await act(async () => void shown!.run()); // "Check again" while the automatic check is still out
  await act(async () => answers[1]("new"));
  await act(async () => answers[0]("old"));
  assert.equal(shown!.data, "new");
  assert.equal(shown!.busy, false);
  await act(async () => root.unmount());
});
