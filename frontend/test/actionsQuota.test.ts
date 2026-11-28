import { test } from "node:test";
import assert from "node:assert/strict";
import { monthName, parseQuota, quotaPercent, quotaTone } from "../src/lib/actionsQuota.ts";

test("the percent of a quota, none without one", () => {
  assert.equal(quotaPercent(700, 1000), 70);
  assert.equal(quotaPercent(700, null), null);
  assert.equal(quotaPercent(700, 0), null);
});

test("the bar turns at the warning lines", () => {
  assert.equal(quotaTone(69.9), "bg-good");
  assert.equal(quotaTone(70), "bg-warn");
  assert.equal(quotaTone(89.9), "bg-warn");
  assert.equal(quotaTone(90), "bg-bad");
  assert.equal(quotaTone(140), "bg-bad");
});

test("a quota field takes whole minutes, empty or 0 for none", () => {
  assert.equal(parseQuota(""), null);
  assert.equal(parseQuota(" 0 "), null);
  assert.equal(parseQuota("3000"), 3000);
  assert.equal(parseQuota("3,000"), 3000);
  assert.equal(parseQuota("1000000"), 1_000_000);
  assert.equal(parseQuota("1000001"), undefined);
  assert.equal(parseQuota("2.5"), undefined);
  assert.equal(parseQuota("-5"), undefined);
  assert.equal(parseQuota("lots"), undefined);
});

test("a month by name", () => {
  assert.equal(monthName("2026-11"), "November 2026");
  assert.equal(monthName("soon"), "soon");
});
