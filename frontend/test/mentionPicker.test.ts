import { test } from "node:test";
import assert from "node:assert/strict";
import { atQuery, insertMention, mentionedIn, mentionGroups } from "../src/lib/mentionPicker.ts";

const a = (name: string, project: string) => ({ id: name.toLowerCase().replace(/\s/g, "-"), name, project }) as never;
const team = [a("Loft Dev", "loft"), a("Loft Dev 2", "loft"), a("Loft QA", "loft"), a("Web Dev", "shop"), a("Planner", "")];
const names = (q: string, project = "loft") => mentionGroups(team, project, q).map((g) => [g.title, g.agents.map((x) => x.name)]);

test("the project's agents come first under its name, the rest under Other agents", () => {
  assert.deepEqual(names(""), [["loft", ["Loft Dev", "Loft Dev 2", "Loft QA"]], ["Other agents", ["Web Dev", "Planner"]]]);
});

test("typing after the @ filters by the start of the name or of a word in it, across both groups", () => {
  assert.deepEqual(names("de"), [["loft", ["Loft Dev", "Loft Dev 2"]], ["Other agents", ["Web Dev"]]]);
  assert.deepEqual(names("QA"), [["loft", ["Loft QA"]]]);
  assert.deepEqual(names("zzz"), []);
});

test("an empty group is left out, and a project-less issue still lists everyone", () => {
  assert.deepEqual(names("pl", "loft"), [["Other agents", ["Planner"]]]);
  assert.deepEqual(names("w", "").map((g) => g[0]), ["Other agents"]);
});

test("the @ query is read at the end of the text, at the start of a word only", () => {
  assert.equal(atQuery("ask @Lo"), "Lo");
  assert.equal(atQuery("@"), "");
  assert.equal(atQuery("(@we"), "we");
  assert.equal(atQuery("mail me@x"), undefined);
  assert.equal(atQuery("@Loft Dev now"), undefined);
});

test("picking inserts the full name the backend resolves, replacing what was typed", () => {
  assert.equal(insertMention("please @Lo", team[0]), "please @Loft Dev ");
  assert.equal(insertMention("@", team[3]), "@Web Dev ");
});

test("a mention counts only where the backend resolves it", () => {
  const who = (text: string) => mentionedIn(text, team).map((x: { name: string }) => x.name);
  assert.deepEqual(who("ping @loft dev, please"), ["Loft Dev"]);
  assert.deepEqual(who("@Loft Dev 2 and @Web Dev."), ["Loft Dev 2", "Web Dev"], "the longer name uses its @ up, Loft Dev is not also told");
  assert.deepEqual(who("@Web Developer"), []);
  assert.deepEqual(who("see @Web Dev/notes.md"), []);
  assert.deepEqual(who("mail me@Planner"), []);
});
