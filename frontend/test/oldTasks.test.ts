import { test } from "node:test";
import assert from "node:assert/strict";
import { oldTasksTarget } from "../src/lib/oldTasks.ts";

test("/tasks goes to /issues and keeps the filters in the query", () => {
  assert.deepEqual(oldTasksTarget(undefined, "?source=github&q=retry"), { pathname: "/issues", search: "?source=github&q=retry" });
});

test("/tasks/:id goes to /issues/:id and keeps the query", () => {
  assert.deepEqual(oldTasksTarget("abc", "?x=1"), { pathname: "/issues/abc", search: "?x=1" });
});
