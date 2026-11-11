// Run with `pnpm test`. Long words must wrap inside every chat bubble (#287).
// There is no DOM renderer here, so this checks the class names in the source.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const chat = readFileSync(new URL("../src/pages/agents/Chat.tsx", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/index.css", import.meta.url), "utf8");

test("every pre-wrapped bubble also breaks long words", () => {
  const classes = [...chat.matchAll(/className="([^"]*whitespace-pre-wrap[^"]*)"/g)].map((m) => m[1]);
  // user, agent→agent (both ways), error and Reasoning, plus the tool output <pre>s
  assert.ok(classes.length >= 5, `found ${classes.length}`);
  for (const c of classes) {
    // the input overlay mirrors the textarea; tool output already uses break-all
    if (c.includes("pointer-events-none") || c.includes("break-all")) continue;
    assert.match(c, /\[overflow-wrap:anywhere\]/, c);
  }
});

test("Markdown replies (own, mention replies, quotes) break long words", () => {
  assert.match(chat, /cx\("prose-chat /, "agent Markdown uses prose-chat");
  const rule = css.match(/\.prose-chat \{([^}]*)\}/);
  assert.ok(rule, ".prose-chat rule");
  assert.match(rule[1], /overflow-wrap: anywhere;/);
});
