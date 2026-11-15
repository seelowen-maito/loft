import { test } from "node:test";
import assert from "node:assert/strict";
import { nextRunText } from "../src/lib/format.ts";

const now = Date.now();
const at = (s: number) => new Date(now + s * 1000).toISOString();

test("a check-in due, overdue or under 45 s away is any moment", () => {
  assert.equal(nextRunText(at(-300), now), "next: any moment");
  assert.equal(nextRunText(at(0), now), "next: any moment");
  assert.equal(nextRunText(at(30), now), "next: any moment");
});

test("a later one says when", () => {
  assert.match(nextRunText(at(300), now), /^next in /);
});
