// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { teamNote } from "../src/lib/teamNote.ts";

test("one agent: Delivery works with it, and every role is suggested", () => {
  assert.equal(
    teamNote(["dev", "code_reviewer", "qa", "security"]),
    "Delivery works with this agent building the tickets. Add a Dev to build them, a QA and a Security agent to have each ticket reviewed, and a Code reviewer for the spec PR.",
  );
});

test("only the missing roles are named", () => {
  assert.equal(teamNote(["code_reviewer", "qa", "security"]), "Delivery works with this team. Add a QA and a Security agent to have each ticket reviewed, and a Code reviewer for the spec PR.");
  assert.equal(teamNote(["security"]), "Delivery works with this team. Add a Security agent to have each ticket reviewed.");
  assert.equal(teamNote(["dev", "qa"]), "Delivery works with this agent building the tickets. Add a Dev to build them, and a QA agent to have each ticket reviewed.");
  assert.equal(teamNote(["code_reviewer"]), "Delivery works with this team. Add a Code reviewer for the spec PR.");
});

test("a full team gets no note", () => {
  assert.equal(teamNote([]), "");
});
