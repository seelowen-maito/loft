import { test } from "node:test";
import assert from "node:assert/strict";
import { canAlwaysAllow } from "../src/lib/permission.ts";

test("a Kill offers Always allow only when Loft made a rule", () => {
  assert.equal(canAlwaysAllow({ tool_name: "Kill", rule: "Kill(/d/*/App)" }), true);
  assert.equal(canAlwaysAllow({ tool_name: "Kill", rule: "" }), false);
});

test("a change to the user's checkout never offers it", () => {
  assert.equal(canAlwaysAllow({ tool_name: "Checkout", rule: "" }), false);
  assert.equal(canAlwaysAllow({ tool_name: "Checkout", rule: "Checkout(/d/loft)" }), false);
});

test("other tools always offer it", () => {
  assert.equal(canAlwaysAllow({ tool_name: "Bash", rule: "Bash(git *)" }), true);
  assert.equal(canAlwaysAllow({ tool_name: "Bash", rule: "" }), true);
});
