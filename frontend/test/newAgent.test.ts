// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { EMPTY_FORM, FRESH_CONTEXT, TEMPLATES, createPayload, offersFreshContext, templateOf, withTeam } from "../src/lib/newAgent.ts";

test("a new agent starts Custom with Fresh context always, from a template too", () => {
  assert.equal(EMPTY_FORM.role, "custom");
  assert.equal(EMPTY_FORM.fresh_context, "always");
  for (const t of TEMPLATES) assert.equal(createPayload({ ...EMPTY_FORM, ...t }).fresh_context, "always", t.label);
});

test("every Fresh context option stays selectable, Always marked as the default", () => {
  assert.deepEqual(FRESH_CONTEXT.map((f) => f.value).sort(), ["above_100k", "above_120k", "above_150k", "above_180k", "above_200k", "above_80k", "always", "off"]);
  assert.deepEqual(FRESH_CONTEXT.filter((f) => f.label.includes("(default)")).map((f) => f.value), ["always"]);
});

test("each template sets its role", () => {
  assert.deepEqual(Object.fromEntries(TEMPLATES.map((t) => [t.label, t.role])), {
    "Repo janitor": "custom",
    "Research scout": "custom",
    "Pair programmer": "dev",
  });
});

test("clearing the group drops a Lead to Custom, projects or not", () => {
  const lead = { ...EMPTY_FORM, group: "Acme", projects: ["loft"], role: "lead" };
  assert.equal(withTeam(lead, { group: "" }).role, "custom");
  assert.equal(withTeam(lead, { group: "  " }).role, "custom", "a blank group counts as none");
  assert.equal(withTeam(lead, { projects: ["web", "api"] }).role, "lead", "other projects keep it Lead");
  assert.equal(withTeam(lead, { projects: [] }).role, "lead", "a group alone is a team");
  assert.equal(withTeam({ ...EMPTY_FORM, projects: ["loft"], role: "qa" }, { projects: [] }).role, "qa", "other roles stay");
});

test("Fresh context is hidden only for a Lead whose team has a Dev", () => {
  const agents = [{ role: "dev", project: "api", group: "Acme" }, { role: "qa", project: "notes" }];
  assert.equal(offersFreshContext({ ...EMPTY_FORM, group: "Acme", projects: ["web"], role: "lead" }, agents), false, "its group has a Dev, in another project");
  assert.equal(offersFreshContext({ ...EMPTY_FORM, group: "Notes", projects: ["notes"], role: "lead" }, agents), true, "no Dev: builds the tickets");
  assert.equal(offersFreshContext({ ...EMPTY_FORM, group: "Api", projects: ["api"], role: "lead" }, agents), true, "a Dev in its project but another group isn't its team's");
  for (const role of ["dev", "code_reviewer", "qa", "security", "custom"]) {
    assert.equal(offersFreshContext({ ...EMPTY_FORM, group: "Acme", role }, agents), true, role);
  }
});

test("the create payload carries group, projects, role and Fresh context", () => {
  const p = createPayload({ ...EMPTY_FORM, name: "  ", group: "Acme", projects: ["web", "api"], role: "lead", fresh_context: "always" });
  assert.equal(p.name, "Unnamed");
  assert.equal(p.role, "lead");
  assert.equal(p.fresh_context, "always");
  assert.deepEqual([p.group, p.projects], ["Acme", ["web", "api"]]);
});

test("a form has no title, and the template it was started from stays highlighted until its instructions change", () => {
  assert.equal("title" in EMPTY_FORM, false);
  for (const t of TEMPLATES) {
    assert.equal("title" in t, false, t.label);
    const f = { ...EMPTY_FORM, ...t };
    assert.equal(templateOf(f), t, t.label);
    assert.equal(templateOf({ ...f, name: "Renamed" }), t, "renaming keeps it");
    assert.equal(templateOf({ ...f, prompt: f.prompt + " And more." }), undefined, "new instructions are the user's own");
  }
  assert.equal(templateOf(EMPTY_FORM), undefined);
});
