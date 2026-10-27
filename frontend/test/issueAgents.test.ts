// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { agentsOnRepo } from "../src/lib/issueAgents.ts";

const projects = [
  { project: "loft", repo: "octo/loft" },
  { project: "loft-copy", repo: "octo/loft" },
  { project: "imole", repo: "octo/imole" },
  { project: "scratch", repo: null },
];
const agents = [
  { name: "reviewer", project: "loft" },
  { name: "Dev", project: "loft-copy" },
  { name: "Imole Dev", project: "imole" },
  { name: "Scratch", project: "scratch" },
  { name: "Loner", project: "" },
];

test("the picker lists only agents on the issue's repo, from any of its folders, A–Z", () => {
  assert.deepEqual(agentsOnRepo(agents, projects, "octo/loft").map((a) => a.name), ["Dev", "reviewer"]);
  assert.deepEqual(agentsOnRepo(agents, projects, "octo/imole").map((a) => a.name), ["Imole Dev"]);
});

test("a repo no project is on lists nobody", () => {
  assert.deepEqual(agentsOnRepo(agents, projects, "octo/elsewhere"), []);
});

test("an agent's project is read trimmed, as the backend reads it", () => {
  assert.deepEqual(agentsOnRepo([{ name: "Padded", project: " loft " }], projects, "octo/loft").map((a) => a.name), ["Padded"]);
});

test("while the agents or the projects are loading there is no answer yet, not an empty one", () => {
  assert.equal(agentsOnRepo(agents, null, "octo/loft"), null);
  assert.equal(agentsOnRepo(null, projects, "octo/loft"), null);
});

test("repo names match ignoring case, as on GitHub", () => {
  assert.deepEqual(agentsOnRepo(agents, [{ project: "loft", repo: "Octo/Loft" }], "octo/loft").map((a) => a.name), ["reviewer"]);
  assert.deepEqual(agentsOnRepo(agents, projects, "OCTO/IMOLE").map((a) => a.name), ["Imole Dev"]);
});
