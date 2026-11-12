import assert from "node:assert/strict";
import { test } from "node:test";
import { restartDisabled } from "../src/lib/restartButton.ts";

const s = (running_agents: number, why: string | null = null) => ({ running_agents, why });

test("the button is on when idle", () => assert.equal(restartDisabled(s(0), false), false));
test("it is off while agents run", () => assert.equal(restartDisabled(s(2, "2 agents are running; restarts when they finish"), false), true));
test("it is off while a restart is in progress", () => assert.equal(restartDisabled(s(0), true), true));
test("a failed restart frees it", () => assert.equal(restartDisabled(s(0, "restart failed: boom"), true), false));
