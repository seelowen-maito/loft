// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { BLANK_CUSTOM, EMPTY_FORM, FRESH_CONTEXT, applyPick, createPayload, currentPick, pickLocked, pickNumber, pickerFooter, pickerKey, refillTemplate, rolePicks, withTeam, type AgentTemplate } from "../src/lib/newAgent.ts";

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

// The picker's lines in its order: Lead, Dev, Code reviewer, QA, Security, Custom, Deps, Docs, Triage, CI.
const rows = () => {
  const { team, solo } = picks();
  return [...team, ...solo];
};
const lineAt = (move: ReturnType<typeof pickerKey>) => (move && "at" in move ? move.at : null);

test("in the picker ↑↓ move the highlight and wrap around", () => {
  const r = rows();
  assert.equal(r.length, 10);
  assert.deepEqual(pickerKey(0, r, "ArrowDown", EMPTY_FORM), { at: 1 });
  assert.deepEqual(pickerKey(4, r, "ArrowUp", EMPTY_FORM), { at: 3 });
  assert.deepEqual(pickerKey(9, r, "ArrowDown", EMPTY_FORM), { at: 0 }, "past the last line: the first");
  assert.deepEqual(pickerKey(0, r, "ArrowUp", EMPTY_FORM), { at: 9 }, "before the first line: the last");
});

test("in the picker 1 to 9 jump to their line and 0 to the tenth; a number past the last line does nothing", () => {
  const r = rows();
  for (let i = 0; i < 10; i++) {
    assert.equal(pickNumber(i), i < 9 ? String(i + 1) : "0");
    assert.deepEqual(pickerKey(5, r, pickNumber(i), EMPTY_FORM), { at: i }, pickNumber(i));
  }
  assert.equal(r[lineAt(pickerKey(0, r, "7", EMPTY_FORM))!].label, "Deps");
  assert.equal(r[lineAt(pickerKey(0, r, "0", EMPTY_FORM))!].label, "CI");
  assert.equal(pickNumber(10), "", "no key after the tenth line");
  const six = r.slice(0, 6);
  for (const k of ["7", "9", "0"]) assert.equal(pickerKey(2, six, k, EMPTY_FORM), null, `${k} with six lines`);
  for (const k of ["a", "Tab", " ", "Escape"]) assert.equal(pickerKey(2, r, k, EMPTY_FORM), null, k);
  assert.equal(pickerKey(0, [], "ArrowDown", EMPTY_FORM), null, "no lines");
});

test("in the picker Enter picks the highlighted line, never a locked Lead", () => {
  const r = rows();
  assert.equal(r[0].role, "lead");
  assert.equal(pickerKey(0, r, "Enter", EMPTY_FORM), null, "Lead without a group");
  assert.deepEqual(pickerKey(0, r, "Enter", { ...EMPTY_FORM, group: "Acme" }), { pick: r[0] }, "Lead with a group");
  assert.deepEqual(pickerKey(6, r, "Enter", EMPTY_FORM), { pick: r[6] });
  assert.equal(r[6].label, "Deps");
});

test("the picker's footer is about the highlighted line, the Lead lock first", () => {
  const r = rows();
  assert.equal(pickerFooter(6, r, EMPTY_FORM), "Deps about");
  assert.equal(pickerFooter(8, r, { ...EMPTY_FORM, role: "custom", prompt: r[6].template!.prompt }), "Triage about", "not the current line's");
  assert.equal(pickerFooter(0, r, EMPTY_FORM), "Only an agent with a group can be Lead: set its group first. Lead about");
  assert.equal(pickerFooter(0, r, { ...EMPTY_FORM, group: "Acme" }), "Lead about");
  assert.equal(pickerFooter(0, [], EMPTY_FORM), "");
});
