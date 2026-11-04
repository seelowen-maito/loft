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

/** True when the `/word` `picked` from the menu is still in `text` as the same whole word at the same place. */
export function stillPicked(text: string, picked: CommandWord | null | undefined): picked is CommandWord {
  return !!picked && slashWords(text).some((w) => w.start === picked.start && w.word === picked.word);
}

/** The message's one command, the `/word` that runs when it's sent: its first word when that is exactly one of
 * `names` (in any case), else the command `picked` (or completed) from the menu while it's still the same word
 * at the same place. A known `/word` typed by hand mid-message ("what does /compact do") is plain text, and
 * so is any `/word` after the command. Null when nothing runs. */
export function messageCommand(text: string, names: string[], picked?: CommandWord | null): CommandWord | null {
  const known = new Set(names.map((n) => n.toLowerCase()));
  const words = slashWords(text);
  const first = words[0];
  if (first && !text.slice(0, first.start).trim() && known.has(first.word.toLowerCase())) return first;
  return stillPicked(text, picked) && known.has(picked.word.toLowerCase()) ? picked : null;
}

/** The command being typed at `caret`: the `/word` that starts a word and holds the caret (anywhere after its
 * `/`), unless another command that runs (see `messageCommand`) is already in the message. Null when there is
 * none, so the menu stays closed. */
export function commandAt(text: string, caret: number, names: string[], picked?: CommandWord | null): CommandWord | null {
  const at = slashWords(text).find((w) => caret > w.start && caret <= w.end);
  if (!at) return null;
  const command = messageCommand(text, names, picked);
  return command && command.start !== at.start ? null : at;
}

/** A message cut into its one command (lowercased, as it runs) and the rest of its text, trimmed, with the
 * command taken out wherever it sat: "the login bug /grill", with `/grill` picked from the menu, is `/grill`
 * with "the login bug". A later `/word` stays in the rest. Null when no command runs (see `messageCommand`). */
export function splitCommand(text: string, names: string[], picked?: CommandWord | null): { command: string; rest: string } | null {
  const c = messageCommand(text, names, picked);
  if (!c) return null;
  const before = text.slice(0, c.start), after = text.slice(c.end);
  const head = before.trimEnd(), tail = after.trimStart();
  // The two sides meet with the whitespace that stood before the command (or after it, at the start).
  const gap = before.slice(head.length) || after.slice(0, after.length - tail.length);
  return { command: c.word.toLowerCase(), rest: (head && tail ? head + gap + tail : head + tail).trim() };
}

/** What Enter does mid-message with the menu open on the `/word` `at` and `name` its highlighted match: "send"
 * only once `at` is the command `picked` from the menu (so it runs and is highlighted), else "pick" it: complete
 * it in place and mark it picked, leaving a second Enter to send. A command typed by hand in full is never run by
 * the Enter that would have sent it as text. */
export function enterOnMenu(text: string, at: CommandWord, name: string, picked?: CommandWord | null): "send" | "pick" {
  return stillPicked(text, picked) && picked.start === at.start && at.word.toLowerCase() === name ? "send" : "pick";
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

/** The composer's `text` cut into pieces, `accent` on its one command that runs (see `messageCommand`) and on
 * each mention of `agents`, for the overlay that colours them. */
export function highlightParts(
  text: string,
  commands: string[],
  agents: string[],
  picked?: CommandWord | null,
): { text: string; accent: boolean }[] {
  const command = messageCommand(text, commands, picked);
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
