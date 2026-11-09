import { test } from "node:test";
import assert from "node:assert/strict";
import { pendingItems } from "../src/lib/pending.ts";
import type { PermissionRequest } from "../src/lib/api.ts";

const req = (id: string, agent_id: string, tool_name: string, created_at: string, status: PermissionRequest["status"] = "pending") =>
  ({ id, agent_id, tool_name, created_at, status, source: "prompt", input: {}, reason: "", rule: "" }) as PermissionRequest;
const agents = [{ id: "a1", name: "Loft Dev" }, { id: "a2", name: "Web QA" }];

test("each pending request links to its card in its agent's chat, and says who wants what", () => {
  assert.deepEqual(pendingItems([req("r1", "a1", "Bash(npm *)", "2026-11-09T10:00:00Z")], agents), [
    { id: "r1", agent: "Loft Dev", tool: "Bash(npm *)", href: "/agents/a1#perm-r1", title: "Loft Dev wants to use Bash(npm *)" },
  ]);
});

test("several: oldest first, decided ones left out", () => {
  const items = pendingItems([
    req("new", "a1", "WebFetch", "2026-11-09T10:05:00Z"),
    req("done", "a1", "Bash", "2026-11-09T09:00:00Z", "allowed_once"),
    req("old", "a2", "mcp__github", "2026-11-09T10:01:00Z"),
  ], agents);
  assert.deepEqual(items.map((i) => i.id), ["old", "new"]);
  assert.equal(items[0].href, "/agents/a2#perm-old");
});

test("an agent that isn't loaded yet still gets a link", () => {
  const [i] = pendingItems([req("r", "gone", "Bash", "2026-11-09T10:00:00Z")], agents);
  assert.equal(i.title, "An agent wants to use Bash");
  assert.equal(i.href, "/agents/gone#perm-r");
});
