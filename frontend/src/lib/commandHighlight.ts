/** How many leading characters of the composer's `text` to colour as a command: the whole `/name` once it
 * exactly matches one of `names` (in any case), whether or not arguments follow, else 0. */
export function commandSpan(text: string, names: string[]): number {
  const word = /^\/\S*/.exec(text)?.[0];
  return word && names.some((n) => n.toLowerCase() === word.toLowerCase()) ? word.length : 0;
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

/** The composer's `text` cut into pieces, `accent` on a known command at its start and on each mention of
 * `agents`, for the overlay that colours them. */
export function highlightParts(text: string, commands: string[], agents: string[]): { text: string; accent: boolean }[] {
  const command = commandSpan(text, commands);
  const ranges = [...(command > 0 ? [[0, command] as [number, number]] : []), ...mentionRanges(text, agents).filter(([s]) => s >= command)];
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
