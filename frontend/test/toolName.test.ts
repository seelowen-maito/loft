import { test } from "node:test";
import assert from "node:assert/strict";
import { shortToolName } from "../src/lib/format.ts";

test("Loft's tool names lose exactly their mcp__loft__ prefix", () => {
  assert.equal(shortToolName("mcp__loft__set_status"), "set_status");
  assert.equal(shortToolName("mcp__loft__update"), "update");
  assert.equal(shortToolName("mcp__github__create_issue"), "github › create_issue");
  assert.equal(shortToolName("Bash"), "Bash");
});
