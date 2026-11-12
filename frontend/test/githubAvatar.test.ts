// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { githubAvatarUrl, githubLogin } from "../src/lib/githubAvatar.ts";

test("a login gives its GitHub picture, asked at twice the size", () => {
  assert.equal(githubAvatarUrl("pedropuppin", 14), "https://github.com/pedropuppin.png?size=28");
});

test("a leading @ and spaces are dropped", () => {
  assert.equal(githubAvatarUrl(" @octo-cat ", 20), "https://github.com/octo-cat.png?size=40");
  assert.equal(githubLogin("@octocat"), "octocat");
});

test("an empty login has no picture", () => {
  assert.equal(githubAvatarUrl("", 14), null);
  assert.equal(githubAvatarUrl("  ", 14), null);
  assert.equal(githubAvatarUrl("@", 14), null);
});

test("odd characters can't break out of the path", () => {
  assert.equal(githubAvatarUrl("a/b?c", 10), "https://github.com/a%2Fb%3Fc.png?size=20");
});
