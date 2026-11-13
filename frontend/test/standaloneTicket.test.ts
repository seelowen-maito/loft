// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { isDeliverableSpec, isStandaloneTicket } from "../src/lib/standaloneTicket.ts";

const ticket = (o: Partial<{ state: string; github: { state: string } | null; labels: string[]; parent_id: string | null; description: string; project: string }> = {}) => ({
  state: "open", github: { state: "open" }, labels: ["ready-for-agent"], parent_id: null, description: "## What to build\n\nA thing.", project: "web", ...o,
});

test("an open ready-for-agent ticket under no spec is standalone", () => {
  assert.equal(isStandaloneTicket(ticket()), true);
});

test("a ticket with a parent is a spec's ticket", () => {
  assert.equal(isStandaloneTicket(ticket({ parent_id: "spec" })), false);
});

test("a ticket naming its spec in the template's Parent section is a spec's ticket", () => {
  assert.equal(isStandaloneTicket(ticket({ description: "## Parent\n\n#229\n\n## What to build\n\nA thing." })), false);
  assert.equal(isStandaloneTicket(ticket({ description: "## What to build\n\nA thing.\n\n## Parent #229" })), false);
});

test("a Parent heading with no number, or Parent mentioned mid-text, doesn't count", () => {
  assert.equal(isStandaloneTicket(ticket({ description: "## Parent\n\nNone yet." })), true);
  assert.equal(isStandaloneTicket(ticket({ description: "See the ## Parent #229 section of the spec." })), true);
});

test("a closed ticket, or one not ready for an agent, isn't started", () => {
  assert.equal(isStandaloneTicket(ticket({ state: "closed" })), false);
  assert.equal(isStandaloneTicket(ticket({ github: { state: "closed" } })), false);
  assert.equal(isStandaloneTicket(ticket({ labels: ["bug"] })), false);
});

test("a Loft issue in a project is standalone too: its Delivery runs in the project's own repo", () => {
  assert.equal(isStandaloneTicket(ticket({ github: null })), true);
  assert.equal(isStandaloneTicket(ticket({ github: null, project: " " })), false, "no project, no repo to build in");
  assert.equal(isStandaloneTicket(ticket({ github: null, state: "closed" })), false);
  assert.equal(isStandaloneTicket(ticket({ github: null, parent_id: "spec" })), false);
});

test("an open spec on GitHub is delivered from its page", () => {
  const spec = (o: Partial<{ state: string; github: { state: string } | null; labels: string[] }> = {}) => ({ state: "open", github: { state: "open" }, labels: ["Spec"], ...o });
  assert.equal(isDeliverableSpec(spec()), true);
  assert.equal(isDeliverableSpec(spec({ labels: ["ready-for-agent"] })), false, "not a spec");
  assert.equal(isDeliverableSpec(spec({ github: null })), false, "a plan kept in Loft starts from its end-of-planning card");
  assert.equal(isDeliverableSpec(spec({ github: { state: "closed" } })), false);
  assert.equal(isDeliverableSpec(spec({ state: "closed" })), false);
});
