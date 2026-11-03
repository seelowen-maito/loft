import { test } from "node:test";
import assert from "node:assert/strict";
import { verdicts } from "../src/lib/verdicts.ts";

test("tests:green and security:passed show as two green chips, in tests, QA, security order", () => {
  assert.deepEqual(verdicts(["security:passed", "bug", "tests:green"]), [
    { label: "tests:green", tone: "good" },
    { label: "security:passed", tone: "good" },
  ]);
});

test("a failure label is a red chip, and wins over a stale pass of the same kind", () => {
  assert.deepEqual(verdicts(["qa:failed"]), [{ label: "qa:failed", tone: "bad" }]);
  assert.deepEqual(verdicts(["tests:green", "tests:failing"]), [{ label: "tests:failing", tone: "bad" }]);
});

test("no verdict labels, or none given, shows no chips", () => {
  assert.deepEqual(verdicts(["bug", "spec"]), []);
  assert.deepEqual(verdicts(undefined), []);
});
