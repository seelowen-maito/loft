// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { browserName } from "../src/lib/pairing.ts";

test("the browser is named with its system, for the person who approves it", () => {
  assert.equal(browserName("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/17 Safari/605.1.15"), "Safari on Mac");
  assert.equal(browserName("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605 Version/17 Mobile Safari/604"), "Safari on iOS");
  assert.equal(browserName("Mozilla/5.0 (X11; Linux x86_64) Chrome/120 Safari/537.36"), "Chrome on Linux");
  assert.equal(browserName("whatever"), "Browser");
});
