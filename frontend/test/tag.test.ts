// Run with `pnpm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { hashToken, insertTag, tagMatches, tagRows } from "../src/lib/shortcuts.ts";
import type { PullRequest, Task } from "../src/lib/api.ts";

const task = (o: Partial<Task> & { title: string }) => ({ id: "11111111-1111-1111-1111-111111111111", labels: [], project: "loft", state: "open", github: null, ...o }) as Task;
const gh = (repo: string, number: number) => ({ repo, number, url: "", state: "open" as const, updated_at: "" });
const pr = (o: Partial<PullRequest>) => ({ state: "open", url: `https://x/${o.number}`, ...o }) as PullRequest;

test("only a # token at the end of the text opens the picker", () => {
  assert.equal(hashToken("see #"), "");
  assert.equal(hashToken("#18"), "18");
  assert.equal(hashToken("fix (#18"), "18");
  assert.equal(hashToken("see #18 and"), undefined, "a finished tag");
  assert.equal(hashToken("color#fff"), undefined, "a # inside a word");
});

test("a pick replaces the token and ends it with a space", () => {
  assert.equal(insertTag("look at #sp", "#199"), "look at #199 ");
});

test("tags are #N in the agent's project, owner/repo#N elsewhere, the id for a Loft-only issue", () => {
  const rows = tagRows(
    [
      task({ title: "Tag", labels: ["spec"], github: gh("o/loft", 199) }),
      task({ title: "Elsewhere", project: "api", github: gh("o/api", 5) }),
      task({ title: "Local", id: "22222222-2222-2222-2222-222222222222" }),
    ],
    [pr({ title: "Spec PR", slug: "o/loft", number: 200 }), pr({ title: "Spec PR", slug: "o/loft", number: 200 })],
    "loft",
  );
  assert.deepEqual(rows.map((r) => [r.kind, r.tag]), [["spec", "#199"], ["issue", "o/api#5"], ["issue", "#22222222-2222-2222-2222-222222222222"], ["pr", "#200"]]);
  assert.equal(tagRows([task({ title: "T", github: gh("o/loft", 1) })], [], "")[0].tag, "o/loft#1", "no project: always the full form");
});

test("matches list specs, issues, then PRs; a number finds that item", () => {
  const rows = tagRows(
    [task({ title: "Tag things", labels: ["spec"], github: gh("o/loft", 199) }), task({ title: "Tag bug", github: gh("o/loft", 7) })],
    [pr({ title: "Tag PR", slug: "o/loft", number: 200 })],
    "loft",
  );
  assert.deepEqual(tagMatches(rows, "tag").map((r) => r.kind), ["spec", "issue", "pr"]);
  assert.deepEqual(tagMatches(rows, "7").map((r) => r.tag), ["#7"]);
  assert.equal(tagMatches(rows, "").length, 3, "nothing typed: the open ones");
});
