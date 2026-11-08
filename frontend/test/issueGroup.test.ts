// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { agentInGroup, groupAgents, groupProjects, inGroup } from "../src/lib/team.ts";

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
  assert.ok(!inGroup({ project: "notes", agent_id: null }, "notes", agents), "no group label: its project is not a group");
  assert.ok(!inGroup({ project: "", agent_id: null }, "Dineout", agents));
});

test("the project choices narrow to the group's projects", () => {
  const known = ["dine/api", "dine/pos", "loft", "notes"];
  assert.deepEqual(groupProjects("Dineout", agents, known), ["dine/api", "dine/pos"]);
  assert.deepEqual(groupProjects("", agents, known), known);
  assert.deepEqual(groupProjects("Nobody", agents, known), []);
});

test("the agent choices narrow to the group's agents", () => {
  assert.deepEqual(groupAgents("Dineout", agents).map((a) => a.id), ["lead", "dev"]);
  assert.deepEqual(groupAgents("", agents).map((a) => a.id), ["lead", "dev", "solo", "loft"], "all groups: every agent");
  assert.deepEqual(groupAgents("Nobody", agents), []);
});

test("switching group keeps an agent in it and clears one outside it", () => {
  assert.equal(agentInGroup("dev", "Dineout", agents), "dev");
  assert.equal(agentInGroup("loft", "Dineout", agents), "", "an agent of another group");
  assert.equal(agentInGroup("solo", "Dineout", agents), "", "an agent with no group");
  assert.equal(agentInGroup("loft", "", agents), "loft", "all groups keeps any agent");
  assert.equal(agentInGroup("gone", "", agents), "", "an unknown agent");
  assert.equal(agentInGroup("", "Dineout", agents), "");
});
