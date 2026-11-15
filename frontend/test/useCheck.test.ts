import { test } from "node:test";
import assert from "node:assert/strict";
import { afterCheck } from "../src/lib/useCheck.ts";

test("a good check replaces the data and clears an earlier error", () => {
  assert.deepEqual(afterCheck({ data: null, err: "boom" }, { ok: 1 }), { data: 1, err: null });
});

test("a failed check keeps the last data and shows its error", () => {
  assert.deepEqual(afterCheck({ data: 1, err: null }, { error: new Error("offline") }), { data: 1, err: "offline" });
});
