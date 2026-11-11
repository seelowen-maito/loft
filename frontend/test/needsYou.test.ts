import { test } from "node:test";
import assert from "node:assert/strict";
import { answerBody, badgeText, badgeTitle, closeTarget, countAfter, escCloses, iconAction, idsChanged, itemView, placeOf, urgencyOf, type NeedsYouItem } from "../src/lib/needsYou.ts";

const HOST = "mac.tail1234.ts.net";

const item = (over: Partial<NeedsYouItem>): NeedsYouItem => ({
  id: "x", type: "question", urgency: 2, agent: null, task_id: null, title: "T", reason: "R", text: "", created_at: "2026-11-11T10:00:00Z",
  age_seconds: 0, links: [], actions: [], project: null, ...over,
});

const stalled = item({
  id: "stalled:o/r:380:2026-11-11T10:00:00Z",
  type: "delivery_stalled",
  agent: { id: "dev1", name: "Loft Dev" },
  task_id: "spec-task",
  reason: "Loft Dev asks: Keep the old route?",
  links: [{ label: "Issue", path: "/issues/spec-task" }],
  actions: [
    { kind: "resume", label: "Resume", method: "POST", path: "/api/deliveries/resume", body: { repo: "o/r", spec: 380 } },
    { kind: "answer", label: "Reply", method: "POST", path: "/api/agents/dev1/messages", body: { text: "" } },
    { kind: "open", label: "Open chat", target: { screen: "chat", agent_id: "dev1" } },
    { kind: "open", label: "Open issue", target: { screen: "task", task_id: "spec-task" } },
    { kind: "dismiss", label: "Dismiss", method: "POST", path: "/api/needs-you/stalled%3Ao/dismiss", body: {} },
  ],
});

test("a stalled Delivery with a question leads to the agent's chat and the Delivery card, with Resume, Reply and Dismiss", () => {
  const v = itemView(stalled, HOST);
  assert.deepEqual(v.open, [
    { label: "Open chat", place: { to: "/agents/dev1" } },
    { label: "Open issue", place: { to: "/issues/spec-task" } },
  ], "the issue link goes to the same place as Open issue, so it shows once");
  assert.deepEqual(v.calls.map((a) => a.kind), ["resume"]);
  assert.equal(v.answer?.path, "/api/agents/dev1/messages");
  assert.equal(v.dismiss?.label, "Dismiss");
  assert.deepEqual(answerBody(v.answer!, "Drop it"), { text: "Drop it" });
});

test("a permission sends its decisions; Allow, Always allow and Deny keep their bodies", () => {
  const v = itemView(item({
    type: "permission",
    actions: [
      { kind: "approve", label: "Allow once", method: "POST", path: "/api/permissions/p1", body: { decision: "allow_once" } },
      { kind: "approve_always", label: "Always allow", method: "POST", path: "/api/permissions/p1", body: { decision: "allow_always", rule: "Bash(x)" } },
      { kind: "deny", label: "Deny", method: "POST", path: "/api/permissions/p1", body: { decision: "deny" } },
      { kind: "open", label: "Open chat", target: { screen: "chat", agent_id: "a1" } },
    ],
  }), HOST);
  assert.deepEqual(v.calls.map((a) => a.body?.decision), ["allow_once", "allow_always", "deny"]);
  assert.equal(v.answer, null);
  assert.equal(v.dismiss, null, "a permission can't be dismissed");
});

test("Loft's own pages on this Mac open here; GitHub and Screen Sharing open outside", () => {
  assert.deepEqual(placeOf({ url: `http://${HOST}:7420/settings#push` }, HOST), { to: "/settings#push" });
  assert.deepEqual(placeOf({ url: "https://github.com/o/r/pull/7" }, HOST), { href: "https://github.com/o/r/pull/7" });
  assert.deepEqual(placeOf({ url: `vnc://${HOST}` }, HOST), { href: `vnc://${HOST}` });
  assert.deepEqual(placeOf({ url: `http://${HOST}:7420/settings#push` }, "localhost"), { href: `http://${HOST}:7420/settings#push` }, "from another host name it opens in a tab");
  assert.deepEqual(placeOf({ screen: "needs_you" }, HOST), { to: "/needs-you" });
  assert.equal(placeOf({ screen: "somewhere_new" }, HOST), null, "an unknown screen is left out");
});

test("a Delivery with no issue to open doesn't link to Needs you from Needs you", () => {
  const v = itemView(item({ type: "delivery_stalled", actions: [{ kind: "open", label: "Open Needs you", target: { screen: "needs_you" } }] }), HOST);
  assert.deepEqual(v.open, []);
});

test("push off points at Settings → Push", () => {
  const v = itemView(item({
    type: "push_off", urgency: 1,
    links: [{ label: "Settings → Push notifications", path: "/settings#push" }],
    actions: [
      { kind: "open", label: "Open Settings", target: { url: `http://${HOST}:7420/settings#push` } },
      { kind: "dismiss", label: "Dismiss", method: "POST", path: "/api/needs-you/push_off/dismiss", body: {} },
    ],
  }), HOST);
  assert.deepEqual(v.open, [{ label: "Open Settings", place: { to: "/settings#push" } }]);
});

test("urgency in words", () => {
  assert.deepEqual([3, 2, 1, 0].map((u) => urgencyOf(u).label), ["Blocks an agent", "Waiting for you", "Worth a look", "To review"]);
});

test("the counter follows needs_you events and ignores the others", () => {
  let c = { count: 2, urgent: 0 };
  c = countAfter(c, { type: "agent_updated" });
  assert.deepEqual(c, { count: 2, urgent: 0 });
  c = countAfter(c, { type: "needs_you", count: 5, urgent: 1 });
  assert.deepEqual(c, { count: 5, urgent: 1 });
  assert.deepEqual(countAfter(c, { type: "needs_you", count: 0, urgent: 0 }), { count: 0, urgent: 0 });
});

test("the counter's number and title", () => {
  assert.deepEqual([0, 1, 99, 100].map(badgeText), ["", "1", "99", "99+"]);
  assert.equal(badgeTitle({ count: 0, urgent: 0 }), "Nothing needs you");
  assert.equal(badgeTitle({ count: 1, urgent: 0 }), "1 thing needs you");
  assert.equal(badgeTitle({ count: 3, urgent: 2 }), "3 things need you, 2 urgent");
});

test("the page reloads only when the ids change", () => {
  assert.equal(idsChanged(["a", "b"], ["a", "b"]), false);
  assert.equal(idsChanged(["a", "b"], ["b", "a"]), true);
  assert.equal(idsChanged(["a"], ["a", "c"]), true);
});

test("the icon opens Needs you from any page and closes it while it's open", () => {
  for (const p of ["/", "/agents/dev1", "/issues", "/needs-you/x"]) assert.equal(iconAction(p), "open", p);
  assert.equal(iconAction("/needs-you"), "close");
});

test("closing (icon, X or Esc) goes back to the page Needs you was opened from, or home when it was opened by its URL", () => {
  assert.equal(closeTarget("k3x9f2"), "back");
  assert.equal(closeTarget("default"), "/");
});

test("Esc closes Needs you, but not while typing an answer, composing, or with a dialog over it", () => {
  const esc = { key: "Escape", defaultPrevented: false, isComposing: false, typing: false, dialogOpen: false };
  assert.equal(escCloses(esc), true);
  assert.equal(escCloses({ ...esc, typing: true }), false);
  assert.equal(escCloses({ ...esc, isComposing: true }), false);
  assert.equal(escCloses({ ...esc, defaultPrevented: true }), false);
  assert.equal(escCloses({ ...esc, dialogOpen: true }), false);
  assert.equal(escCloses({ ...esc, key: "Enter" }), false);
});
