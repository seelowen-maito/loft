// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { CONTEXT_WARN_AT, contextWarns, listedContext } from "../src/lib/context.ts";

test("the context size warns from 80k tokens", () => {
  assert.equal(CONTEXT_WARN_AT, 80_000);
  assert.equal(contextWarns(0), false);
  assert.equal(contextWarns(79_999), false, "below 80k is neutral");
  assert.equal(contextWarns(80_000), true, "80k itself warns");
  assert.equal(contextWarns(250_000), true);
});

test("the agents list shows the size only for a Session that has replied", () => {
  assert.equal(listedContext({ session_id: "s", context_tokens: 167_000 }), 167_000);
  assert.equal(listedContext({ session_id: "s", context_tokens: null }), null, "before its first reply");
  assert.equal(listedContext({ session_id: "s" }), null, "an older server");
  assert.equal(listedContext({ session_id: null, context_tokens: 167_000 }), null, "no Session, as after Clear");
});
