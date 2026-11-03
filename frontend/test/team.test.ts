import { test } from "node:test";
import assert from "node:assert/strict";
import { groupByTeam, outsideWarning, projectChoices, projectsOf, teamOf, withProject } from "../src/lib/team.ts";

const a = (name: string, group: string | undefined, project: string, projects?: string[]) => ({ name, group, project, projects });

test("an agent's team is its group, else its main project", () => {
  assert.equal(teamOf(a("Web", "Dineout", "b2c")), "Dineout");
  assert.equal(teamOf(a("Old", undefined, " api ")), "api", "a server without groups");
  assert.equal(teamOf(a("Loose", " ", "")), "");
});

test("its projects come from the list, else from the main project alone", () => {
  assert.deepEqual(projectsOf(a("Web", "Dineout", "b2c", ["b2c", "b2b"])), ["b2c", "b2b"]);
  assert.deepEqual(projectsOf(a("Old", undefined, "api")), ["api"]);
  assert.deepEqual(projectsOf(a("Loose", "", "", [])), []);
});

test("the list is grouped by team, A–Z, those without one last", () => {
  const list = [a("Loose", "", ""), a("Web", "dineout", "b2c"), a("Api", "Dineout", "api"), a("Isa", "", "imole"), a("App", "Dineout", "apps")];
  assert.deepEqual(groupByTeam(list).map((g) => [g.team, g.agents.map((x) => x.name)]), [
    ["dineout", ["Web"]],
    ["Dineout", ["Api", "App"]],
    ["imole", ["Isa"]],
    ["", ["Loose"]],
  ]);
});

test("adding a project appends it once; starring makes it the main one", () => {
  assert.deepEqual(withProject(["b2c"], " b2b "), ["b2c", "b2b"]);
  assert.deepEqual(withProject(["b2c", "b2b"], "b2c"), ["b2c", "b2b"], "already there: unchanged");
  assert.deepEqual(withProject(["b2c", "b2b"], "b2b", true), ["b2b", "b2c"]);
  assert.deepEqual(withProject(["b2c"], ""), ["b2c"]);
});

test("a new issue offers the agent's projects first and every other known one after", () => {
  const web = a("Web", "Dineout", "b2c", ["b2c", "b2b"]);
  assert.deepEqual(projectChoices(web, ["api", "b2b", " apps ", "", "api"]), { mine: ["b2c", "b2b"], others: ["api", "apps"] });
  assert.deepEqual(projectChoices(undefined, ["b2c"]), { mine: [], others: ["b2c"] });
});

test("a project outside the agent's warns, but only when it has projects", () => {
  const web = a("Web", "Dineout", "b2c", ["b2c", "b2b"]);
  assert.equal(outsideWarning(web, "b2b"), null);
  assert.equal(outsideWarning(web, ""), null);
  assert.equal(outsideWarning(a("Loose", "", "", []), "api"), null);
  assert.match(outsideWarning(web, "api") ?? "", /^Web doesn't work in api \(its projects: b2c, b2b\)/);
});
