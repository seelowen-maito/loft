// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { loadLastAgent, openingAgent, saveLastAgent, selectedAgent } from "../src/lib/lastAgent.ts";

/** An in-memory localStorage; a fresh `loadLastAgent` on it stands in for a reload. */
function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

const agents = [{ id: "lead" }, { id: "dev" }, { id: "reviewer" }];

test("with nothing remembered it opens the first agent", () => {
  assert.equal(openingAgent(agents, loadLastAgent(memory())), "lead");
});

test("it opens the last agent viewed, across a reload", () => {
  const s = memory();
  saveLastAgent("dev", s);
  saveLastAgent("reviewer", s);
  assert.equal(openingAgent(agents, loadLastAgent(s)), "reviewer");
});

test("a deleted last agent falls back to the first", () => {
  const s = memory();
  saveLastAgent("gone", s);
  assert.equal(openingAgent(agents, loadLastAgent(s)), "lead");
});

test("no agents opens none", () => {
  assert.equal(openingAgent([], "dev"), null);
});

test("unavailable storage falls back to the first agent", () => {
  const blocked = { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } };
  saveLastAgent("dev", blocked);
  assert.equal(openingAgent(agents, loadLastAgent(blocked)), "lead");
});

test("the Agents tab opens the agent just viewed while the page stays mounted", () => {
  const s = memory();
  assert.equal(selectedAgent(undefined, agents, s), "lead");
  saveLastAgent(selectedAgent("dev", agents, s)!, s);
  assert.equal(selectedAgent(undefined, agents, s), "dev", "/agents/dev → /agents opens dev");
});

test("deleting the selected agent opens the first", () => {
  const s = memory();
  saveLastAgent("dev", s);
  assert.equal(selectedAgent(undefined, agents.filter((a) => a.id !== "dev"), s), "lead");
});

test("before the agents load nothing is selected unless the route names one", () => {
  assert.equal(selectedAgent(undefined, null, memory()), null);
  assert.equal(selectedAgent("dev", null, memory()), "dev");
});
