// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { latestOnly } from "../src/lib/latest.ts";

/** A promise the test settles by hand, to choose the order answers arrive in. */
function deferred<T>() {
  let resolve!: (v: T) => void, reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
  return { promise, resolve, reject };
}
const settle = () => new Promise((r) => setTimeout(r, 0));

test("an older answer arriving after a newer one is ignored", async () => {
  const shown: string[] = [];
  const l = latestOnly<string>((v) => shown.push(v));
  const older = deferred<string>(), newer = deferred<string>();
  l.load(older.promise);
  l.load(newer.promise);
  newer.resolve("running");
  await settle();
  older.resolve("stalled");
  await settle();
  assert.deepEqual(shown, ["running"]);
});

test("answers that arrive in order are each shown", async () => {
  const shown: string[] = [];
  const l = latestOnly<string>((v) => shown.push(v));
  const a = deferred<string>();
  l.load(a.promise);
  a.resolve("stalled");
  await settle();
  const b = deferred<string>();
  l.load(b.promise);
  b.resolve("running");
  await settle();
  assert.deepEqual(shown, ["stalled", "running"]);
});

test("a Resume's own answer beats a reload that started before it", async () => {
  const shown: string[] = [];
  const l = latestOnly<string>((v) => shown.push(v));
  const reload = deferred<string>();
  l.load(reload.promise);
  l.set("running");
  reload.resolve("stalled");
  await settle();
  assert.deepEqual(shown, ["running"]);
});

test("nothing is shown after the owner is gone, until it starts again", async () => {
  const shown: string[] = [];
  const l = latestOnly<string>((v) => shown.push(v));
  const inFlight = deferred<string>();
  l.load(inFlight.promise);
  l.stop();
  inFlight.resolve("stalled");
  l.set("paused");
  await settle();
  assert.deepEqual(shown, []);
  l.start();
  l.load(Promise.resolve("running"));
  await settle();
  assert.deepEqual(shown, ["running"]);
});

test("a failed load shows nothing and doesn't throw", async () => {
  const shown: string[] = [];
  const l = latestOnly<string>((v) => shown.push(v));
  const d = deferred<string>();
  l.load(d.promise);
  d.reject(new Error("offline"));
  await settle();
  assert.deepEqual(shown, []);
});
