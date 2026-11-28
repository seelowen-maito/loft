import { test } from "node:test";
import assert from "node:assert/strict";
import { fmtQuiet } from "../src/lib/format.ts";

test("fmtQuiet says when the agent works, or that it never wakes", () => {
  assert.equal(fmtQuiet([{ days: [0, 1, 2, 3, 4], start: "09:00", end: "17:00" }, { days: [5, 6], start: "00:00", end: "00:00" }]), "Works Mon–Fri 09:00–17:00; Sat, Sun all day");
  assert.equal(fmtQuiet([{ days: [], start: "00:00", end: "00:00" }]), "Never wakes on its own");
  assert.equal(fmtQuiet([{ days: [], start: "00:00", end: "00:00" }, { days: [0], start: "09:00", end: "17:00" }]), "Works Mon 09:00–17:00");
});
