import { test } from "node:test";
import assert from "node:assert/strict";
import { issuePickerOrder } from "../src/lib/team.ts";

const a = (name: string, role: string, projects: string[]) => ({ name, role, project: projects[0] ?? "", projects });
const names = (list: { name: string }[], project?: string) => issuePickerOrder(list, project).map((x) => x.name);

test("on a loft issue, Loft Dev comes before Loft Code Review", () => {
  const list = [a("Loft Code Review", "code_reviewer", ["loft"]), a("Loft Dev", "dev", ["loft"])];
  assert.deepEqual(names(list, "loft"), ["Loft Dev", "Loft Code Review"]);
});

test("the project's agents come first, Devs first in each part, then by name", () => {
  const list = [
    a("Zed QA", "qa", ["loft"]),
    a("Api Dev", "dev", ["api"]),
    a("Alpha Lead", "lead", ["api"]),
    a("Loft Dev", "dev", ["loft"]),
    a("Bee Dev", "dev", ["api", "loft"]),
    a("Loose", "", []),
  ];
  assert.deepEqual(names(list, "loft"), ["Bee Dev", "Loft Dev", "Zed QA", "Api Dev", "Alpha Lead", "Loose"]);
});

test("an issue with no project lists Devs first", () => {
  const list = [a("Loft Code Review", "code_reviewer", ["loft"]), a("Api Dev", "dev", ["api"]), a("Loft Dev", "dev", ["loft"])];
  assert.deepEqual(names(list), ["Api Dev", "Loft Dev", "Loft Code Review"]);
  assert.deepEqual(names(list, "  "), ["Api Dev", "Loft Dev", "Loft Code Review"]);
});

test("it leaves the list it was given alone", () => {
  const list = [a("B", "qa", []), a("A", "dev", [])];
  issuePickerOrder(list, "");
  assert.deepEqual(list.map((x) => x.name), ["B", "A"]);
});
