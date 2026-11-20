// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { appendDraft, chipLabel, chips, listeningLabel, MAX_SECONDS, micBlocked, next, pcm16k, sections, START, starter, talkSession, type Step, type TalkAgent, type TalkEvent } from "../src/lib/talk.ts";

const agent = (id: string, more: Partial<TalkAgent> = {}): TalkAgent => ({ id, name: id, last_run_at: null, ...more });
const run = (events: TalkEvent[], from: Step = START) => events.reduce(next, from);

test("talking then sending goes through every step, as on the Mac panel", () => {
  let s = run([{ type: "listening" }]);
  assert.deepEqual(s, { kind: "listening", heard: "" });
  // The text appears while talking.
  s = run([{ type: "heard", text: "please review" }, { type: "heard", text: "please review the PR" }], s);
  assert.deepEqual(s, { kind: "listening", heard: "please review the PR" });
  s = run([{ type: "stop" }, { type: "final", text: "please review the PR um" }], s);
  assert.deepEqual(s, { kind: "cleaning", heard: "please review the PR um" });
  s = run([{ type: "cleaned", text: "Please review the PR." }], s);
  assert.deepEqual(s, { kind: "review", text: "Please review the PR." });
  // The review text can be corrected before it goes.
  s = run([{ type: "edit", text: "Please review the PR. " }, { type: "send" }], s);
  assert.deepEqual(s, { kind: "sending", text: "Please review the PR." });
  assert.deepEqual(run([{ type: "sent" }], s), { kind: "sent" });
});

test("a failed send keeps the text and Retry sends it again", () => {
  const s = run([{ type: "failed", message: "offline" }], { kind: "sending", text: "Hi." });
  assert.deepEqual(s, { kind: "failed", message: "offline", text: "Hi." });
  assert.deepEqual(next(s, { type: "retry" }), { kind: "sending", text: "Hi." });
});

test("a failure while listening starts over on Try again", () => {
  const s = run([{ type: "listening" }, { type: "failed", message: "Transcribing needs macOS 26." }]);
  assert.deepEqual(s, { kind: "failed", message: "Transcribing needs macOS 26." });
  assert.deepEqual(next(s, { type: "retry" }), START);
});

test("nothing heard fails instead of sending an empty message", () => {
  const s = run([{ type: "listening" }, { type: "stop" }, { type: "final", text: "  " }]);
  assert.equal(s.kind, "failed");
});

test("late answers don't move a step they don't belong to", () => {
  const review: Step = { kind: "review", text: "Hi." };
  assert.deepEqual(run([{ type: "heard", text: "late" }, { type: "final", text: "late" }, { type: "cleaned", text: "x" }], review), review);
  assert.deepEqual(next(START, { type: "send" }), START);
  assert.deepEqual(next({ kind: "review", text: " " }, { type: "send" }), { kind: "review", text: " " }, "nothing to send");
});

test("Talk starts with the given agent, else the last one, else a Lead, else the first", () => {
  const agents = [agent("dev"), agent("lead", { role: "lead" })];
  assert.equal(starter(agents, "dev", "lead"), "dev");
  assert.equal(starter(agents, null, "dev"), "dev");
  assert.equal(starter(agents, "gone", "gone"), "lead");
  assert.equal(starter([agent("a"), agent("b")], null, null), "a");
  assert.equal(starter([], null, null), null);
});

test("the To chips: current, every Lead, then the most recently run up to the limit", () => {
  const agents = [
    agent("old", { last_run_at: "2026-01-01T00:00:00Z" }),
    agent("new", { last_run_at: "2026-03-01T00:00:00Z" }),
    agent("mid", { last_run_at: "2026-02-01T00:00:00Z" }),
    agent("lead1", { role: "lead" }),
    agent("lead2", { role: "lead", last_run_at: "2026-01-15T00:00:00Z" }),
    agent("me"),
  ];
  assert.deepEqual(chips(agents, "me", 4).map((a) => a.id), ["me", "lead2", "lead1", "new"]);
  // Leads are all shown, even past the limit.
  assert.deepEqual(chips(agents, "me", 2).map((a) => a.id), ["me", "lead2", "lead1"]);
});

test("a chip shows its group when another chip has the same name", () => {
  const a = agent("1", { name: "Lead", group: "Dineout" });
  const b = agent("2", { name: "Lead", group: "Loft" });
  assert.equal(chipLabel(a, [a, b]), "Lead · Dineout");
  assert.equal(chipLabel(a, [a]), "Lead");
});

test("More lists every agent by group and searches name, role, group and project", () => {
  const agents = [
    agent("b", { name: "Bea", group: "Loft", projects: ["loft"] }),
    agent("a", { name: "al", group: "Loft", role: "qa" }),
    agent("z", { name: "Zed" }),
    agent("c", { name: "Cy", group: "Acme", project: "api" }),
  ];
  assert.deepEqual(sections(agents, "").map((s) => [s.title, s.agents.map((a) => a.name)]), [
    ["Acme", ["Cy"]],
    ["Loft", ["al", "Bea"]],
    ["No group", ["Zed"]],
  ]);
  assert.deepEqual(sections(agents, "API").map((s) => s.agents.map((a) => a.id)), [["c"]]);
  assert.deepEqual(sections(agents, "qa").map((s) => s.agents.map((a) => a.id)), [["a"]]);
  assert.deepEqual(sections(agents, "nobody"), []);
});

test("the mic says why it's off on plain http", () => {
  assert.match(micBlocked(false, true) ?? "", /https/);
  assert.equal(micBlocked(true, true), null);
  assert.ok(micBlocked(true, false));
});

test("audio becomes 16 kHz 16-bit little-endian PCM", () => {
  // 48 kHz: three input samples per output sample, averaged.
  const out = pcm16k(new Float32Array([1, 1, 1, -1, -1, -1, 0.5, 0.5, 0.5]), 48000);
  assert.equal(out.length, 6);
  const v = new DataView(out.buffer);
  assert.deepEqual([v.getInt16(0, true), v.getInt16(2, true), v.getInt16(4, true)], [32767, -32767, 16384]);
  assert.equal(pcm16k(new Float32Array(16000), 16000).length, 32000, "one second");
});

test("Edit in chat adds the text under what the message box already holds", () => {
  assert.equal(appendDraft("", "Hi."), "Hi.");
  assert.equal(appendDraft("First line  ", "Hi."), "First line\nHi.");
});

/** A fake Mac: each call waits until the test answers it, with the text "<bytes so far> bytes". */
function fakeMac() {
  const calls: { audio: number[]; final: boolean; answer: () => void; fail: (m: string) => void }[] = [];
  let total = 0;
  const transcribe = (_id: string, audio: Uint8Array, final: boolean) =>
    new Promise<{ text: string }>((resolve, reject) => {
      total += audio.length;
      const text = `${total} bytes`;
      calls.push({ audio: [...audio], final, answer: () => resolve({ text }), fail: (m) => reject(new Error(m)) });
    });
  return { calls, transcribe };
}
const settle = () => new Promise((r) => setTimeout(r, 0));

test("chunks go in order, one at a time; ticks while one is out wait for the next chunk", async () => {
  const mac = fakeMac();
  let mic = [1, 2];
  const heard: string[] = [];
  const s = talkSession("s1", mac.transcribe, () => Uint8Array.from(mic.splice(0)), (t) => heard.push(t), () => assert.fail("no failure"));
  s.tick(1);
  await settle();
  mic.push(3, 4);
  s.tick(2); // the first chunk is still out: this audio waits
  assert.equal(mac.calls.length, 1);
  mac.calls[0].answer();
  await settle();
  mic.push(5, 6);
  s.tick(3); // both waiting blocks go together
  await settle();
  assert.deepEqual(mac.calls[1].audio, [3, 4, 5, 6]);
  // Done while a chunk is out: the final chunk goes after it, and its text is the answer.
  mic.push(7, 8);
  const last = s.finish();
  await settle();
  assert.equal(mac.calls.length, 2, "the final chunk waits for the one before");
  mac.calls[1].answer();
  await settle();
  assert.deepEqual(mac.calls[2], { ...mac.calls[2], audio: [7, 8], final: true });
  mac.calls[2].answer();
  assert.equal(await last, "8 bytes");
  assert.deepEqual(heard, ["2 bytes", "6 bytes", "8 bytes"]);
  s.tick(4);
  s.discard();
  assert.equal(mac.calls.length, 3, "nothing goes after the session ended");
});

test("cancel deletes the session's audio on the Mac, and a late answer is ignored", async () => {
  const mac = fakeMac();
  const heard: string[] = [];
  const failed: string[] = [];
  const s = talkSession("s1", mac.transcribe, () => Uint8Array.of(1, 2), (t) => heard.push(t), (e) => failed.push(e.message));
  s.tick(1);
  await settle();
  s.discard();
  assert.deepEqual(mac.calls[1], { ...mac.calls[1], audio: [], final: true });
  mac.calls[0].answer();
  await settle();
  assert.deepEqual(heard, []);
  assert.deepEqual(failed, []);
});

test("a failed chunk is reported once", async () => {
  const mac = fakeMac();
  const failed: string[] = [];
  const s = talkSession("s1", mac.transcribe, () => Uint8Array.of(1, 2), () => {}, (e) => failed.push(e.message));
  s.tick(1);
  await settle();
  mac.calls[0].fail("Transcribing timed out.");
  await settle();
  assert.deepEqual(failed, ["Transcribing timed out."]);
});

test("Talk stops itself before the Mac's ten-minute limit, and says so in the last minute", () => {
  const mac = fakeMac();
  const s = talkSession("s1", mac.transcribe, () => Uint8Array.of(1, 2), () => {}, () => {});
  assert.equal(s.tick(MAX_SECONDS - 1), null);
  assert.equal(s.tick(MAX_SECONDS), "stop");
  // The Mac refuses more than ten minutes of 16 kHz 16-bit audio (voice.rs MAX_AUDIO_BYTES).
  assert.ok(MAX_SECONDS * 16000 * 2 < 10 * 60 * 16000 * 2);
  assert.equal(listeningLabel(12), "Listening · 12s");
  assert.equal(listeningLabel(MAX_SECONDS - 45), "Listening · stops in 45s");
});
