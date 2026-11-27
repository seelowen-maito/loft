// The shared `/api/info`: read once for every component, every subscriber told when it arrives, and a failed read
// retried by the next `ensure` (the live stream opening again) instead of leaving it unknown until a reload.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createSharedValue } from "../src/lib/sharedValue.ts";

/** A fake loader whose answers the test settles one by one. */
function loader() {
  const calls: { resolve: (v: string) => void; reject: (e: Error) => void }[] = [];
  const load = () => new Promise<string>((resolve, reject) => calls.push({ resolve, reject }));
  return { calls, load };
}
const tick = () => new Promise((r) => setTimeout(r));

test("concurrent readers share one load, and every subscriber hears the answer", async () => {
  const { calls, load } = loader();
  const v = createSharedValue(load);
  let a = 0;
  let b = 0;
  v.subscribe(() => a++);
  v.subscribe(() => b++);
  v.ensure();
  v.ensure();
  assert.equal(calls.length, 1);
  calls[0].resolve("mac");
  await tick();
  assert.equal(v.get(), "mac");
  assert.deepEqual([a, b], [1, 1]);
  v.ensure();
  assert.equal(calls.length, 1, "loaded: no read again");
});

test("a failed load leaves it unknown, and the next ensure retries and reaches the components already mounted", async () => {
  const { calls, load } = loader();
  const v = createSharedValue(load);
  let heard = 0;
  v.subscribe(() => heard++);
  v.ensure();
  calls[0].reject(new Error("backend restarting"));
  await tick();
  assert.equal(v.get(), null);
  assert.equal(heard, 0);
  v.ensure();
  assert.equal(calls.length, 2);
  calls[1].resolve("mac");
  await tick();
  assert.equal(v.get(), "mac");
  assert.equal(heard, 1);
});

test("a reload keeps the value it has until the new one arrives, and keeps it on a failure", async () => {
  const { calls, load } = loader();
  const v = createSharedValue(load);
  v.ensure();
  calls[0].resolve("no token");
  await tick();
  v.reload();
  assert.equal(v.get(), "no token");
  calls[1].reject(new Error("offline"));
  await tick();
  assert.equal(v.get(), "no token");
  v.reload();
  calls[2].resolve("token");
  await tick();
  assert.equal(v.get(), "token");
});
