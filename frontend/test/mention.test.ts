// Run with `npm test` (Node's own test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { atToken, matchingEntries, mentionMode, withPick } from "../src/lib/mention.ts";

test("only an @ token at the end of the text opens the menu", () => {
  assert.deepEqual(atToken("see @"), { query: "", dir: "", prefix: "" });
  assert.deepEqual(atToken("@Web"), { query: "web", dir: "", prefix: "web" });
  assert.equal(atToken("see @src/main.rs and"), undefined, "a finished pick");
  assert.equal(atToken("mail me@home"), undefined, "an @ inside a word");
  assert.equal(atToken("no token"), undefined);
});

test("the folder part is kept as typed, the entry part matches in any case", () => {
  assert.deepEqual(atToken("look at @../imole/Pro"), { query: "../imole/pro", dir: "../imole/", prefix: "pro" });
});

const entries = [
  { name: "src", dir: true },
  { name: "README.md", dir: false },
  { name: "Readme.old", dir: false },
  { name: ".env", dir: false },
];

test("entries match the typed start, hidden ones only once a dot is typed", () => {
  assert.deepEqual(matchingEntries(entries, "").map((e) => e.name), ["src", "README.md", "Readme.old"]);
  assert.deepEqual(matchingEntries(entries, "read").map((e) => e.name), ["README.md", "Readme.old"]);
  assert.deepEqual(matchingEntries(entries, ".").map((e) => e.name), [".env"]);
});

test("picking a folder keeps the menu on its contents, a file or agent ends the token", () => {
  assert.equal(withPick("see @sr", { dir: "", entry: { name: "src", dir: true } }), "see @src/");
  assert.equal(withPick("see @src/ma", { dir: "src/", entry: { name: "main.rs", dir: false } }), "see @src/main.rs ");
  assert.equal(withPick("@we", { agent: "Web Dev" }), "@Web Dev ");
  assert.equal(atToken(withPick("see @sr", { dir: "", entry: { name: "src", dir: true } }))?.dir, "src/");
});

test("a name searches the whole project, a path or nothing yet browses folder by folder", () => {
  const mode = (text: string) => mentionMode(atToken(text)!.query);
  assert.equal(mode("see @chat"), "search");
  assert.equal(mode("@Chat.tsx"), "search");
  assert.equal(mode("@.github"), "search", "a hidden name searches too");
  assert.equal(mode("see @"), "browse", "nothing typed lists the project's top folder");
  assert.equal(mode("@frontend/"), "browse");
  assert.equal(mode("@frontend/src/Ch"), "browse");
  assert.equal(mode("@."), "browse", "on the way to ../");
  assert.equal(mode("@.."), "browse");
  assert.equal(mode("@../imole/pro"), "browse");
});

test("a search pick puts its whole path in the token", () => {
  assert.equal(withPick("see @chat", { dir: "", entry: { name: "frontend/src/pages/agents/Chat.tsx", dir: false } }), "see @frontend/src/pages/agents/Chat.tsx ");
  const folder = withPick("see @agents", { dir: "", entry: { name: "frontend/src/pages/agents", dir: true } });
  assert.equal(folder, "see @frontend/src/pages/agents/", "a folder stays open");
  assert.equal(mentionMode(atToken(folder)!.query), "browse", "and browses its contents");
});
