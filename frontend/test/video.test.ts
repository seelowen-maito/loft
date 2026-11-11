// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { attachable, fmtLength, isVideo, MAX_VIDEO_BYTES } from "../src/lib/video.ts";

const f = (type: string, size = 1000) => ({ type, size });

test("the chat takes images and MP4, MOV and WebM videos", () => {
  const all = [f("image/png"), f("video/mp4"), f("video/quicktime"), f("video/webm")];
  assert.deepEqual(attachable(all, true), { ok: all, why: null });
});

test("other videos, and videos over 200 MB, are left out with a reason", () => {
  assert.deepEqual(attachable([f("video/x-matroska"), f("image/png")], true), { ok: [f("image/png")], why: "Only images and MP4, MOV or WebM videos can be attached." });
  assert.deepEqual(attachable([f("video/mp4", MAX_VIDEO_BYTES + 1)], true), { ok: [], why: "Videos can be up to 200 MB." });
  assert.equal(attachable([f("video/mp4", MAX_VIDEO_BYTES)], true).ok.length, 1);
});

test("where videos aren't taken (grill answers), only images are", () => {
  assert.deepEqual(attachable([f("video/mp4"), f("image/jpeg")], false), { ok: [f("image/jpeg")], why: "Only images can be attached." });
});

test("stored uploads are told apart by name, and lengths read as m:ss", () => {
  assert.equal(isVideo("2f1c4a8e-1d7b-4c43-9a43-0f6f0d7c1a11.mov"), true);
  assert.equal(isVideo("2f1c4a8e-1d7b-4c43-9a43-0f6f0d7c1a11.png"), false);
  assert.equal(fmtLength(7.4), "0:07");
  assert.equal(fmtLength(754), "12:34");
  assert.equal(fmtLength(Infinity), "");
});
