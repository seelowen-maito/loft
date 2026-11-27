// The Help menu's Glossary: the terms of GLOSSARY.md, read from the file itself.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseGlossary } from "../src/lib/glossary.ts";
import { goKeys } from "../src/lib/shortcuts.ts";

test("each term has its section, its meaning on one line and what to avoid", () => {
  const md = "# Loft\n\nIntro.\n\n## Agents and work\n\n**Agent**:\nA long-lived worker\nwith a name.\n_Avoid_: Bot, assistant\n\n**Waiting** (ticket):\nA ticket that waits.\n\n## Push\n\n**Relay**: The hosted push service.\n";
  assert.deepEqual(parseGlossary(md), [
    { section: "Agents and work", term: "Agent", meaning: "A long-lived worker with a name.", avoid: "Bot, assistant" },
    { section: "Agents and work", term: "Waiting (ticket)", meaning: "A ticket that waits.", avoid: "" },
    { section: "Push", term: "Relay", meaning: "The hosted push service.", avoid: "" },
  ]);
});

test("the repo's GLOSSARY.md parses: every term has a meaning", () => {
  const terms = parseGlossary(readFileSync(new URL("../../GLOSSARY.md", import.meta.url), "utf8"));
  assert.ok(terms.length > 20, `${terms.length} terms`);
  assert.deepEqual(terms.filter((t) => !t.meaning || !t.section).map((t) => t.term), []);
  assert.ok(terms.some((t) => t.term === "Delivery"));
});

test("a tab's tooltip names its g shortcut", () => {
  assert.equal(goKeys("/agents"), "G A");
  assert.equal(goKeys("/settings"), "G S");
  assert.equal(goKeys("/saved"), undefined);
});
