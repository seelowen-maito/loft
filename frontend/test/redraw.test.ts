import assert from "node:assert/strict";
import { test } from "node:test";
import { pendingClick, redrawBusy, redrawClick } from "../src/lib/redraw.ts";
import type { Agent } from "../src/lib/api.ts";

const sticker = (svg: string) => ({ svg }) as unknown as Agent["avatar"];
const agent = (redrawing: boolean | undefined, svg = "<old/>") => ({ redrawing, avatar: sticker(svg) });

test("Redraw is clickable when nothing is queued or drawing and nothing was clicked", () => {
  assert.equal(redrawBusy(agent(false), null), false);
  assert.equal(redrawBusy(agent(undefined), null), false, "a server without the flag");
});

test("a click shows busy straight away, before the agent's event arrives", () => {
  const click = redrawClick(agent(false));
  assert.equal(redrawBusy(agent(false), click), true);
  assert.notEqual(pendingClick(click, agent(false)), null, "the click still waits for the agent");
});

test("once the agent says it's redrawing, the flag takes over from the click", () => {
  const click = redrawClick(agent(false));
  assert.equal(pendingClick(click, agent(true)), null);
  assert.equal(redrawBusy(agent(true), null), true, "queued or drawing, after a reload or in another tab");
});

test("when the Redraw ends, drawn or failed, the button is clickable again", () => {
  // The click was handed over to the flag, which is now false.
  assert.equal(redrawBusy(agent(false, "<new/>"), null), false, "drawn");
  assert.equal(redrawBusy(agent(false), null), false, "failed or stopped: same sticker, flag false");
});

test("a new sticker settles a click even if the flag never showed true (an older server)", () => {
  const click = redrawClick(agent(undefined));
  assert.equal(pendingClick(click, agent(undefined, "<new/>")), null);
  assert.equal(redrawBusy(agent(undefined, "<new/>"), click), false);
});
