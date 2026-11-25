/** Saved: chat messages kept for good, with a title and a tag (spec #675). */

/** The tags the Save form offers; any other text works too. */
export const SUGGESTED_TAGS = ["Tutorial", "Good to know", "Decision"];

const TITLE_CHARS = 80;

/** The Save form's title for a message: its first line without Markdown's heading, quote or list mark, cut at a word. */
export function defaultTitle(content: string): string {
  const line = content.split("\n").map((l) => l.replace(/^\s*(#{1,6}\s+|>\s*|[-*+]\s+|\d+[.)]\s+)/, "").trim()).find(Boolean) ?? "";
  const chars = [...line];
  if (chars.length <= TITLE_CHARS) return line;
  const cut = chars.slice(0, TITLE_CHARS).join("");
  const space = cut.lastIndexOf(" ");
  return `${(space > 0 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/** The tag filter's choices: the suggested tags, then the others in use, each once whatever its case. */
export function tagOptions(used: string[]): string[] {
  const seen = new Set<string>();
  return [...SUGGESTED_TAGS, ...used].filter((t) => {
    const k = t.trim().toLowerCase();
    if (!k || seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}

/** Where a saved item's back-link goes: its message in the agent's chat. */
export const sourceHref = (i: { agent_id: string; message_id: number }) => `/agents/${encodeURIComponent(i.agent_id)}#msg-${i.message_id}`;

/** The message a chat link's `#msg-<id>` points at, if any. */
export function linkedMessage(hash: string): number | null {
  const m = /^#msg-(\d+)$/.exec(hash);
  return m ? Number(m[1]) : null;
}

/** The list's one-line preview of an item's content: plain words, without Markdown's marks. */
export function excerpt(content: string, chars = 160): string {
  // A table's separator row and a horizontal rule aren't words.
  // One character class, not `\s*\|?\s*`: two `\s*` in a row backtrack in quadratic time on a long run of spaces (#681).
  const text = content.split("\n").filter((l) => !/^[\s|]*:?-{3,}/.test(l)).join("\n");
  const words = text.replace(/[`*_>#|[\]]/g, " ").split(/\s+/).filter(Boolean).join(" ");
  const c = [...words];
  return c.length > chars ? `${c.slice(0, chars).join("")}…` : words;
}

/** What an agent's message mentions a saved item as: `#saved-<id>-<title words>`, which the backend reads by the id. */
export function savedTag(i: { id: number; title: string }): string {
  const slug = i.title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40).replace(/-+$/, "");
  return slug ? `#saved-${i.id}-${slug}` : `#saved-${i.id}`;
}

/** The items the `#` menu offers for what's typed after the `#`: nothing unless it starts with `saved`, then the
 * words after `saved-` narrow the titles (or the tag). */
export function savedMatches<T extends { id: number; title: string; tag: string }>(items: T[], query: string, limit = 8): T[] {
  const m = /^saved(?:-(.*))?$/i.exec(query);
  if (!m) return [];
  const words = (m[1] ?? "").toLowerCase().split("-").filter(Boolean);
  return items.filter((i) => words.every((w) => `${i.title} ${i.tag}`.toLowerCase().includes(w))).slice(0, limit);
}
