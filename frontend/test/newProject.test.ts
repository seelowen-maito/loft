// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  TRY_IDEAS, createError, defaultLead, fixName, formView, leadsOf, loadProjectDraft, nameProblem, projectPath, saveProjectDraft,
  type FormInput, type NameCheck,
} from "../src/lib/newProject.ts";

test("spaces and capitals are fixed as you type, like the backend's fix_name", () => {
  assert.equal(fixName("On Call"), "on-call");
  assert.equal(fixName("on "), "on-", "a trailing dash stays while typing");
  assert.equal(fixName("on ", false), "on", "and goes in the final name");
  assert.equal(fixName("  My  App!!", false), "my-app");
  assert.equal(fixName("api_v2.web"), "api_v2.web");
  assert.equal(fixName("a/b"), "a-b");
});

test("only what the fix can't repair is invalid", () => {
  assert.equal(nameProblem("on-call"), null);
  for (const bad of ["", ".", "..", ".hidden", "a..b", "x".repeat(101)]) assert.ok(nameProblem(bad), bad);
});

test("the path shows the projects folder under ~", () => {
  assert.equal(projectPath("/Users/renato/projects", "on-call"), "~/projects/on-call");
  assert.equal(projectPath("/srv/code/", "x"), "/srv/code/x");
  assert.equal(projectPath(undefined, "x"), "~/projects/x");
});

test("Try chips are example sentences, not templates", () => {
  for (const t of TRY_IDEAS) assert.ok(t.idea.endsWith("."), t.label);
});

test("the Lead defaults to the last one talked to, else the first in team order", () => {
  const agents = [
    { id: "d", name: "Dev", role: "dev" },
    { id: "a", name: "A", role: "lead", group: "Acme" },
    { id: "b", name: "B", role: "lead", group: "Beta" },
  ];
  const leads = leadsOf(agents);
  assert.deepEqual(leads.map((l) => l.id), ["a", "b"]);
  assert.equal(defaultLead(leads, "b"), "b");
  assert.equal(defaultLead(leads, "d"), "a", "the last agent isn't a Lead");
  assert.equal(defaultLead(leads, null), "a");
  assert.equal(defaultLead([], "b"), null, "no Lead");
});

const ok = (name: string): NameCheck => ({ name, path: `~/projects/${name}`, problem: null });
const base: FormInput = { name: "on-call", idea: "A rota", lead: "a", check: ok("on-call"), touched: false, creating: false };

test("empty: Create waits for both Name and Idea, and untouched fields show no error", () => {
  assert.deepEqual(formView({ ...base, name: "", idea: "", check: null }), { nameError: null, taken: null, canCreate: false });
  assert.equal(formView({ ...base, idea: "  " }).canCreate, false);
  assert.equal(formView(base).canCreate, true);
});

test("an invalid name shows only after blur or submit, and blocks Create", () => {
  const bad = { ...base, name: ".hidden", check: { name: ".hidden", path: "", problem: { kind: "invalid" as const, message: "dot" } } };
  assert.equal(formView(bad).nameError, null, "untouched");
  assert.equal(formView(bad).canCreate, false);
  assert.equal(formView({ ...bad, touched: true }).nameError, "A name can't start with a dot.");
});

test("a taken name shows at once and blocks Create", () => {
  const msg = "~/projects/loft already exists. Pick another name, or open it from Repos.";
  const v = formView({ ...base, name: "Loft", check: { name: "loft", path: "~/projects/loft", problem: { kind: "taken", message: msg } } });
  assert.equal(v.taken, msg);
  assert.equal(v.canCreate, false);
});

test("Create waits for the check of the name now typed", () => {
  assert.equal(formView({ ...base, check: null }).canCreate, false, "still checking");
  assert.equal(formView({ ...base, name: "on-call-2" }).canCreate, false, "the check is for an older name");
});

test("no Lead, or creating, blocks Create", () => {
  assert.equal(formView({ ...base, lead: null }).canCreate, false);
  assert.equal(formView({ ...base, creating: true }).canCreate, false);
});

test("the error says what failed, that nothing was created, and what to do", () => {
  assert.equal(
    createError("~/projects/x", "~/projects/x already exists. Pick another name, or open it from Repos."),
    "Couldn't create ~/projects/x: ~/projects/x already exists. Nothing was created. Pick another name.",
  );
  assert.equal(createError("~/projects/x", "Failed to fetch"), "Couldn't create ~/projects/x: failed to fetch. Nothing was created. Check that Loft is running, then try again.");
  assert.match(createError("~/projects/x", "No Lead yet. A Lead plans new projects with you."), /no Lead yet\. Nothing was created\. Create a Lead/);
});

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k), m };
}

test("unsent text is kept as a Draft, and an empty one is removed", () => {
  const s = memory();
  assert.deepEqual(loadProjectDraft(s), { name: "", idea: "" });
  saveProjectDraft({ name: "on-call", idea: "" }, s);
  assert.deepEqual(loadProjectDraft(s), { name: "on-call", idea: "" });
  saveProjectDraft({ name: " ", idea: "" }, s);
  assert.equal(s.m.size, 0);
  const broken = { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); }, removeItem: () => {} };
  assert.deepEqual(loadProjectDraft(broken), { name: "", idea: "" });
  saveProjectDraft({ name: "x", idea: "y" }, broken);
});
