import { test } from "node:test";
import assert from "node:assert/strict";
import { sendState } from "../src/lib/agentMessage.ts";

const queued = "Queued: Web QA is busy and will read it when its current run ends (message 42).";

test("a queued message shows as queued while the recipient holds it, then delivered", () => {
  assert.equal(sendState(queued, [{ queued_messages: [] }, { queued_messages: [41, 42] }]), "Queued: Web QA is busy");
  assert.equal(sendState(queued, [{ queued_messages: [41] }, {}]), "Delivered");
});

test("a message to an idle agent, an FYI, an error or a pending call shows nothing", () => {
  for (const r of ["Delivered. Web QA is reading it now.", "Delivered as FYI. Web QA will read it at the start of its next run.", "Not sent: kind is required.", undefined]) {
    assert.equal(sendState(r, [{ queued_messages: [42] }]), null, String(r));
  }
});
