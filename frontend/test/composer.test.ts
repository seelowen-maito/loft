// Run with `pnpm test`. The area under the chat box (#713): a short placeholder, one hint line with its keys in
// <kbd>, spend kept on a phone, no session id, and tooltips on the + and issue buttons. Like chatWrap, it reads the source.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const chat = readFileSync(new URL("../src/pages/agents/Chat.tsx", import.meta.url), "utf8");
const under = chat.slice(chat.indexOf("One line of the keys people need"), chat.indexOf("est. ${agent.total_cost_usd") + 80);

test("the placeholder is only Message {agent}…", () => {
  assert.match(chat, /: `Message \$\{agent\.name\}…`\n/);
  assert.doesNotMatch(chat, /for commands, @ to mention/);
});

test("the placeholder stays on one line and ends in an ellipsis", () => {
  assert.match(chat, /className=\{cx\("relative block w-full resize-none text-ellipsis [^"]*placeholder:truncate/);
});

test("one hint line with its keys in kbd, hidden on a phone; spend stays and the session id goes", () => {
  assert.ok(under.length > 100, "the hint line is there");
  assert.deepEqual([...under.matchAll(/<Key k="(.)" \/> (\w[\w ]*)/g)].map((m) => `${m[1]} ${m[2].trim()}`), ["/ commands", "@ mention", "# tag", "? all shortcuts"]);
  assert.match(under, /<span className="flex items-center gap-1\.5 max-md:hidden">/, "the hints hide on a phone");
  assert.match(under, /<span className="ml-auto" title="API list-price equivalent">est\. \$\{agent\.total_cost_usd/, "spend isn't hidden");
  assert.doesNotMatch(chat, /session \$\{agent\.session_id|"new session"/);
  assert.doesNotMatch(chat, /@ agents and files/);
});

test("the + and issue buttons have tooltips that open upward, and keep their aria-labels", () => {
  assert.match(chat, /<Tip up left name="Attach images or videos"[^>]*>\s*<button[\s\S]{0,200}aria-label="Attach images or videos"/);
  assert.match(chat, /<Tip up left name=\{ATTACH\}[^>]*>\s*<button[\s\S]{0,200}aria-label=\{ATTACH\}/);
});
