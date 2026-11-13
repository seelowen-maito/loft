import { test } from "node:test";
import assert from "node:assert/strict";
import { groupByTeam, teamBadge, outsideWarning, projectChoices, projectsOf, teamOf, withProject, withTyped } from "../src/lib/team.ts";

const a = (name: string, group: string | undefined, project: string, projects?: string[]) => ({ name, group, project, projects });

test("an agent's team is its group; a project alone makes none", () => {
  assert.equal(teamOf(a("Web", "Acme", "b2c")), "Acme");
  assert.equal(teamOf(a("Old", undefined, " api ")), "", "a server without groups");
  assert.equal(teamOf(a("Loose", " ", "")), "");
});

test("its projects come from the list, else from the main project alone", () => {
  assert.deepEqual(projectsOf(a("Web", "Acme", "b2c", ["b2c", "b2b"])), ["b2c", "b2b"]);
  assert.deepEqual(projectsOf(a("Old", undefined, "api")), ["api"]);
  assert.deepEqual(projectsOf(a("Loose", "", "", [])), []);
});

test("the list is grouped by team, A–Z, those without one last", () => {
  const list = [a("Loose", "", ""), a("Web", "acme", "b2c"), a("Api", "Acme", "api"), a("Ida", "", "notes"), a("App", "Acme", "apps")];
  assert.deepEqual(groupByTeam(list).map((g) => [g.team, g.agents.map((x) => x.name)]), [
    ["acme", ["Web"]],
    ["Acme", ["Api", "App"]],
    ["", ["Loose", "Ida"]],
  ]);
});

test("adding a project appends it once; starring makes it the main one", () => {
  assert.deepEqual(withProject(["b2c"], " b2b "), ["b2c", "b2b"]);
  assert.deepEqual(withProject(["b2c", "b2b"], "b2c"), ["b2c", "b2b"], "already there: unchanged");
  assert.deepEqual(withProject(["b2c", "b2b"], "b2b", true), ["b2b", "b2c"]);
  assert.deepEqual(withProject(["b2c"], ""), ["b2c"]);
});

test("saving takes in the project still typed in the add field", () => {
  const form = { name: "A", projects: ["b2c"] };
  assert.deepEqual(withTyped(form, " b2b "), { name: "A", projects: ["b2c", "b2b"] });
  assert.deepEqual(withTyped(form, "  "), form, "nothing typed: unchanged");
  assert.deepEqual(withTyped(form, "b2c"), form, "already there: unchanged");
  assert.deepEqual(withTyped({ name: "A", projects: [] }, "notes").projects, ["notes"]);
});

test("a new issue offers the agent's projects first and every other known one after", () => {
  const web = a("Web", "Acme", "b2c", ["b2c", "b2b"]);
  assert.deepEqual(projectChoices(web, ["api", "b2b", " apps ", "", "api"]), { mine: ["b2c", "b2b"], others: ["api", "apps"] });
  assert.deepEqual(projectChoices(undefined, ["b2c"]), { mine: [], others: ["b2c"] });
});

test("a project outside the agent's warns, but only when it has projects", () => {
  const web = a("Web", "Acme", "b2c", ["b2c", "b2b"]);
  assert.equal(outsideWarning(web, "b2b"), null);
  assert.equal(outsideWarning(web, ""), null);
  assert.equal(outsideWarning(a("Loose", "", "", []), "api"), null);
  assert.match(outsideWarning(web, "api") ?? "", /^Web doesn't work in api \(its projects: b2c, b2b\)/);
});

test("the header badge is the group, the main project and +N; the tip lists them all", () => {
  const web = a("Web", "acme", "api", ["api", "web", "docs", "infra", "site", "ios"]);
  assert.deepEqual(teamBadge(web), { text: "acme · api · +5", compact: "+5", all: ["api", "web", "docs", "infra", "site", "ios"] });
  assert.equal(teamBadge(a("One", "acme", "api", ["api"])).text, "acme · api");
  assert.equal(teamBadge(a("Same", "api", "api", ["api"])).text, "api", "a group named after its only project says it once");
  assert.equal(teamBadge(a("One", "acme", "api", ["api"])).compact, "", "without others a phone shows no badge");
  assert.equal(teamBadge(a("Old", undefined, "api")).text, "api · +0".replace(" · +0", ""));
});
