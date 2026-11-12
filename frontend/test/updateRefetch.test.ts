import assert from "node:assert/strict";
import { test } from "node:test";
import { refetchUntilNew } from "../src/lib/updateRefetch.ts";

const st = (phase: string, why: string | null = null) => ({ phase, why }) as never;
const run = async (answers: unknown[], tries = 30) => {
  let calls = 0;
  const seen: unknown[] = [];
  await refetchUntilNew(
    async () => {
      const a = answers[Math.min(calls++, answers.length - 1)];
      if (a instanceof Error) throw a;
      return a as never;
    },
    (s) => seen.push(s),
    tries,
    async () => {},
  );
  return { calls, seen };
};

test("it stops once the phase leaves restart, riding out a process that is down", async () => {
  const { calls } = await run([st("restart"), new Error("down"), st("up_to_date")]);
  assert.equal(calls, 3);
});

test("it stops on a failed restart", async () => {
  const { calls } = await run([st("restart"), st("restart", "restart failed: boom")]);
  assert.equal(calls, 2);
});

test("it gives up after its tries", async () => {
  const { calls } = await run([st("restart")], 30);
  assert.equal(calls, 30);
});
