// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { EMPTY_FORM, TEMPLATES, createPayload, offersFreshContext, withProject } from "../src/lib/newAgent.ts";

test("a new agent starts Custom with Fresh context above 80k", () => {
  assert.equal(EMPTY_FORM.role, "custom");
  assert.equal(EMPTY_FORM.fresh_context, "above_80k");
});

test("each template sets its role", () => {
  assert.deepEqual(Object.fromEntries(TEMPLATES.map((t) => [t.label, t.role])), {
    "Repo janitor": "custom",
    "Research scout": "custom",
    "Pair programmer": "dev",
  });
});

test("clearing the project drops a Lead to Custom", () => {
  const lead = { ...EMPTY_FORM, project: "loft", role: "lead" };
  assert.equal(withProject(lead, "").role, "custom");
  assert.equal(withProject(lead, "  ").role, "custom", "blank counts as no project");
  assert.equal(withProject(lead, "imole").role, "lead", "another project keeps it Lead");
  assert.equal(withProject({ ...EMPTY_FORM, project: "loft", role: "qa" }, "").role, "qa", "other roles stay");
});

test("Fresh context is hidden only for a Lead whose project has a Dev", () => {
  const agents = [{ role: "dev", project: "loft" }, { role: "qa", project: "imole" }];
  assert.equal(offersFreshContext({ ...EMPTY_FORM, project: "loft", role: "lead" }, agents), false, "coordinates");
  assert.equal(offersFreshContext({ ...EMPTY_FORM, project: "imole", role: "lead" }, agents), true, "no Dev: builds the tickets");
  for (const role of ["dev", "code_reviewer", "qa", "security", "custom"]) {
    assert.equal(offersFreshContext({ ...EMPTY_FORM, project: "loft", role }, agents), true, role);
  }
});

test("the create payload carries role and Fresh context", () => {
  const p = createPayload({ ...EMPTY_FORM, name: "  ", project: "loft", role: "lead", fresh_context: "always" });
  assert.equal(p.name, "Unnamed");
  assert.equal(p.role, "lead");
  assert.equal(p.fresh_context, "always");
  assert.equal(p.project, "loft");
});
