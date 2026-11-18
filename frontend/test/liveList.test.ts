import { test } from "node:test";
import assert from "node:assert/strict";
import { createLiveList } from "../src/hooks/liveList.ts";

test("a resync event loads the list again; other events don't", async () => {
  let loads = 0;
  const list = createLiveList<{ id: string }>(async () => [{ id: String(++loads) }], (items) => items);
  list.ensure();
  await list.reload();
  assert.equal(loads, 2);
  list.event({ type: "memory", agent_id: "a" });
  assert.equal(loads, 2, "no extra reload under normal load");
  list.event({ type: "resync" });
  await list.reload();
  assert.equal(loads, 4, "the resync reloaded it (plus our own reload)");
});
