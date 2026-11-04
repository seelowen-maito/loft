/** A `/word` in the composer's text: `[start, end)`, from its `/` to the next whitespace or the end. */
export type CommandWord = { start: number; end: number; word: string };

/** Every `/word` of `text` that starts a word: at the start of the text or right after whitespace. A `/`
 * inside a word ("src/app", "a/b") starts none. */
function slashWords(text: string): CommandWord[] {
  const words: CommandWord[] = [];
  for (const m of text.matchAll(/(?<=^|\s)\/\S*/g)) {
    words.push({ start: m.index, end: m.index + m[0].length, word: m[0] });
  }
  return words;
}

/** The message's one command: its first `/word` that is exactly one of `names` (in any case), or null. Any
 * later `/word` is plain text. */
export function messageCommand(text: string, names: string[]): CommandWord | null {
  const known = new Set(names.map((n) => n.toLowerCase()));
  return slashWords(text).find((w) => known.has(w.word.toLowerCase())) ?? null;
}

/** The command being typed at `caret`: the `/word` that starts a word and holds the caret (anywhere after its
 * `/`), unless another known command is already in the message. Null when there is none, so the menu stays
 * closed. */
export function commandAt(text: string, caret: number, names: string[]): CommandWord | null {
  const at = slashWords(text).find((w) => caret > w.start && caret <= w.end);
  if (!at) return null;
  const command = messageCommand(text, names);
  return command && command.start !== at.start ? null : at;
}

/** A message cut into its one command (lowercased, as it runs) and the rest of its text, trimmed, with the
 * command taken out wherever it sat: "the login bug /grill" is `/grill` with "the login bug". A later `/word`
 * stays in the rest. Null when the message holds no known command. */
export function splitCommand(text: string, names: string[]): { command: string; rest: string } | null {
  const c = messageCommand(text, names);
  if (!c) return null;
  const before = text.slice(0, c.start), after = text.slice(c.end);
  const head = before.trimEnd(), tail = after.trimStart();
  // The two sides meet with the whitespace that stood before the command (or after it, at the start).
  const gap = before.slice(head.length) || after.slice(0, after.length - tail.length);
  return { command: c.word.toLowerCase(), rest: (head && tail ? head + gap + tail : head + tail).trim() };
}

/** `text` with the `/word` `at` replaced by `name`, and where the caret goes: right after it. */
export function completeCommand(text: string, at: CommandWord, name: string): { text: string; caret: number } {
  return { text: text.slice(0, at.start) + name + text.slice(at.end), caret: at.start + name.length };
}

/** The `[start, end)` ranges of `text` that mention one of `agents` by name, by the backend's rules
 * (mentions.rs `parse`): case-insensitive, at an `@` that starts a word, longest name first, ending at a
 * word boundary ("@Web Dev," but not "@Web Devx"), and not going on as a file path ("@qa/x", "@QA.txt"). */
export function mentionRanges(text: string, agents: string[]): [number, number][] {
  const names = agents.filter((n) => n.trim()).sort((a, b) => b.length - a.length);
  const path = (c: string | undefined) => !!c && /[\p{L}\p{N}_\-/.]/u.test(c);
  const ranges: [number, number][] = [];
  for (let i = 0; i < text.length; i++) {
    if (text[i] !== "@" || (i > 0 && !/[\s([{"']/.test(text[i - 1]))) continue;
    const hit = names.find((n) => {
      const end = i + 1 + n.length;
      if (text.slice(i + 1, end).toLowerCase() !== n.toLowerCase()) return false;
      const after = text[end];
      if (after === undefined) return true;
      if (after === "/") return false;
      if (after === ".") return !path(text[end + 1]);
      return !/[\p{L}\p{N}_-]/u.test(after);
    });
    if (hit) {
      ranges.push([i, i + 1 + hit.length]);
      i += hit.length;
    }
  }
  return ranges;
}

/** The composer's `text` cut into pieces, `accent` on its one known command (wherever it sits) and on each
 * mention of `agents`, for the overlay that colours them. */
export function highlightParts(text: string, commands: string[], agents: string[]): { text: string; accent: boolean }[] {
  const command = messageCommand(text, commands);
  const ranges = [...(command ? [[command.start, command.end] as [number, number]] : []), ...mentionRanges(text, agents)]
    .sort((a, b) => a[0] - b[0]);
  const parts: { text: string; accent: boolean }[] = [];
  let at = 0;
  for (const [s, e] of ranges) {
    if (s > at) parts.push({ text: text.slice(at, s), accent: false });
    parts.push({ text: text.slice(s, e), accent: true });
    at = e;
  }
  if (at < text.length) parts.push({ text: text.slice(at), accent: false });
  return parts;
}
