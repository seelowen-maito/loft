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
