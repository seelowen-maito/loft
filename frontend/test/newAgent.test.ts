// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { BLANK_CUSTOM, EMPTY_FORM, FRESH_CONTEXT, applyPick, createPayload, currentPick, pickLocked, refillTemplate, rolePicks, withTeam, type AgentTemplate } from "../src/lib/newAgent.ts";

// What `GET /api/agent-templates` serves, in its order, cut down.
const tpl = (key: string, name: string, role: string, model = "sonnet", interval_minutes = 0, project = "site"): AgentTemplate => ({
  key, name, role, model, interval_minutes, summary: `${name} summary`, about: `${name} about`, personality: `${name} personality`, prompt: `${name} works on **${project}**.`,
});
const served = (project = "site") => [
  tpl("dev", "Dev", "dev", "sonnet", 0, project),
  tpl("lead", "Lead", "lead", "opus", 0, project),
  tpl("reviewer", "Reviewer", "code_reviewer", "sonnet", 240, project),
  tpl("security", "Security", "security", "sonnet", 1440, project),
  tpl("tests", "QA", "qa", "sonnet", 720, project),
  tpl("deps", "Deps", "custom", "sonnet", 10080, project),
  tpl("docs", "Docs", "custom", "sonnet", 1440, project),
  tpl("triage", "Triage", "custom", "haiku", 180, project),
  tpl("ci", "CI", "custom", "haiku", 120, project),
];
const LABELS: Record<string, string> = { lead: "Lead", dev: "Dev", code_reviewer: "Code reviewer", qa: "QA", security: "Security" };
const picks = (t = served()) => rolePicks(t, (r) => LABELS[r] ?? r);

test("a new agent starts Custom and sends no Fresh context, from a template too: the backend starts it Always (#495)", () => {
  assert.equal(EMPTY_FORM.role, "custom");
  assert.ok(!("fresh_context" in EMPTY_FORM));
  const { team, solo } = picks();
  for (const p of [...team, ...solo]) assert.ok(!("fresh_context" in createPayload(applyPick(EMPTY_FORM, p, served()))), p.key);
});

test("the picker shows the team in order, then blank Custom and the Custom templates", () => {
  const { team, solo } = picks();
  assert.deepEqual(team.map((p) => [p.label, p.role]), [["Lead", "lead"], ["Dev", "dev"], ["Code reviewer", "code_reviewer"], ["QA", "qa"], ["Security", "security"]]);
  assert.deepEqual(solo.map((p) => p.label), ["Custom", "Deps", "Docs", "Triage", "CI"]);
  assert.equal(solo[0], BLANK_CUSTOM);
  for (const p of [...team, ...solo]) assert.ok(p.summary && p.about, p.key);
  assert.deepEqual(picks([]), { team: [], solo: [BLANK_CUSTOM] }, "before the templates load");
});

test("picking fills name, instructions, personality, schedule and model, which stay editable", () => {
  const { team, solo } = picks();
  const f = applyPick({ ...EMPTY_FORM, group: "Acme", projects: ["site"] }, solo[4], served());
  assert.deepEqual([f.name, f.role, f.prompt, f.personality, f.model, f.interval_minutes], ["CI", "custom", "CI works on **site**.", "CI personality", "haiku", 120]);
  assert.deepEqual([f.group, f.projects], ["Acme", ["site"]], "the rest of the form stays");
  const edited = { ...f, name: "Hawk", interval_minutes: 60 };
  assert.equal(currentPick(edited, [...team, ...solo]).key, "ci", "renaming or rescheduling keeps it the template");
  assert.equal(currentPick({ ...edited, prompt: "My own." }, [...team, ...solo]), BLANK_CUSTOM, "own instructions make it blank Custom");
  const reviewer = applyPick(f, team[2], served());
  assert.deepEqual([reviewer.name, reviewer.role, reviewer.model, reviewer.interval_minutes], ["Reviewer", "code_reviewer", "sonnet", 240]);
  assert.equal(currentPick({ ...reviewer, prompt: "Changed." }, [...team, ...solo]).key, "reviewer", "a team role stays its role");
});

test("blank Custom clears a template's fields and keeps a name the user typed", () => {
  const { solo } = picks();
  const fromDocs = applyPick(EMPTY_FORM, solo[2], served());
  const blank = applyPick(fromDocs, BLANK_CUSTOM, served());
  assert.deepEqual([blank.name, blank.role, blank.prompt, blank.personality, blank.model, blank.interval_minutes], ["", "custom", "", "", "sonnet", 0]);
  assert.equal(applyPick({ ...fromDocs, name: "Moss" }, BLANK_CUSTOM, served()).name, "Moss");
});

test("Lead is locked until the agent has a group", () => {
  const { team } = picks();
  assert.equal(pickLocked(team[0], EMPTY_FORM), true);
  assert.equal(pickLocked(team[0], { ...EMPTY_FORM, group: "  " }), true, "a blank group counts as none");
  assert.equal(pickLocked(team[0], { ...EMPTY_FORM, group: "Acme" }), false);
  assert.ok(team.slice(1).every((p) => !pickLocked(p, EMPTY_FORM)));
});

test("instructions still a template's follow a new main project; the user's own stay", () => {
  const { solo } = picks();
  const f = applyPick(EMPTY_FORM, solo[1], served("site"));
  assert.equal(refillTemplate(f, served("site"), served("api")).prompt, "Deps works on **api**.");
  const own = { ...f, prompt: "Mine." };
  assert.equal(refillTemplate(own, served("site"), served("api")), own);
  assert.equal(refillTemplate(EMPTY_FORM, [], served("api")), EMPTY_FORM, "the first load leaves an empty form alone");
});

test("every Fresh context option stays selectable, Always marked as the default", () => {
  assert.deepEqual(FRESH_CONTEXT.map((f) => f.value).sort(), ["above_100k", "above_120k", "above_150k", "above_180k", "above_200k", "above_80k", "always", "off"]);
  assert.deepEqual(FRESH_CONTEXT.filter((f) => f.label.includes("(default)")).map((f) => f.value), ["always"]);
});

test("clearing the group drops a Lead to Custom, projects or not", () => {
  const lead = { ...EMPTY_FORM, group: "Acme", projects: ["loft"], role: "lead" };
  assert.equal(withTeam(lead, { group: "" }).role, "custom");
  assert.equal(withTeam(lead, { group: "  " }).role, "custom", "a blank group counts as none");
  assert.equal(withTeam(lead, { projects: ["web", "api"] }).role, "lead", "other projects keep it Lead");
  assert.equal(withTeam(lead, { projects: [] }).role, "lead", "a group alone is a team");
  assert.equal(withTeam({ ...EMPTY_FORM, projects: ["loft"], role: "qa" }, { projects: [] }).role, "qa", "other roles stay");
});

test("the create payload carries group, projects and role", () => {
  const p = createPayload({ ...EMPTY_FORM, name: "  ", group: "Acme", projects: ["web", "api"], role: "lead" });
  assert.equal(p.name, "Unnamed");
  assert.equal(p.role, "lead");
  assert.deepEqual([p.group, p.projects], ["Acme", ["web", "api"]]);
});

test("a form has no title", () => {
  assert.equal("title" in EMPTY_FORM, false);
});
