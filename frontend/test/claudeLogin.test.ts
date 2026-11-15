import { test } from "node:test";
import assert from "node:assert/strict";
import { afterCheck } from "../src/lib/claudeLogin.ts";
import type { ClaudeLogin } from "../src/lib/api.ts";

const ok = { logged_in: true } as ClaudeLogin;

test("a good check clears an earlier error", () => {
  assert.deepEqual(afterCheck({ s: null, err: "boom" }, ok), { s: ok, err: null });
});

test("a failed check keeps the last status and shows its error", () => {
  assert.deepEqual(afterCheck({ s: ok, err: null }, new Error("offline")), { s: ok, err: "offline" });
});
