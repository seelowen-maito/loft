// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { groupProjects, inGroup } from "../src/lib/team.ts";

const agents = [
  { id: "lead", project: "dine/api", group: "Dineout", projects: ["dine/api", "dine/web"] },
  { id: "dev", project: "dine/pos", group: "Dineout", projects: ["dine/pos"] },
  { id: "solo", project: "notes", projects: ["notes"] },
  { id: "loft", project: "loft", group: "Loft", projects: ["loft"] },
];

test("an issue is in the group of the agents that work in its project, or of its assignee", () => {
  assert.ok(inGroup({ project: "dine/web", agent_id: null }, "Dineout", agents));
  assert.ok(inGroup({ project: "dine/pos", agent_id: null }, "Dineout", agents));
  assert.ok(!inGroup({ project: "loft", agent_id: null }, "Dineout", agents));
  assert.ok(inGroup({ project: "elsewhere", agent_id: "dev" }, "Dineout", agents), "assigned to one of its agents");
  assert.ok(inGroup({ project: "notes", agent_id: null }, "notes", agents), "no group label: its main project is its group");
  assert.ok(!inGroup({ project: "", agent_id: null }, "Dineout", agents));
});

test("the project choices narrow to the group's projects", () => {
  const known = ["dine/api", "dine/pos", "loft", "notes"];
  assert.deepEqual(groupProjects("Dineout", agents, known), ["dine/api", "dine/pos"]);
  assert.deepEqual(groupProjects("", agents, known), known);
  assert.deepEqual(groupProjects("Nobody", agents, known), []);
});
