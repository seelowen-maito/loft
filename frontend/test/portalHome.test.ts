// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { portalHome } from "../src/lib/api.ts";

test("the portal is at the relay's own address", () => {
  assert.equal(portalHome({ relay_url: "https://relay.example.com", signin: true }), "https://relay.example.com/");
});

test("a relay without a portal, or no relay, offers only pasting a key", () => {
  assert.equal(portalHome({ relay_url: "https://relay.example.com", signin: false }), null);
  assert.equal(portalHome({ relay_url: null, signin: false }), null);
});
