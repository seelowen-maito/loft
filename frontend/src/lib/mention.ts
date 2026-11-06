/** The `@` token typed at the end of the text: `@ho` or `@../imole/pro`. `dir` is the folder part typed so far
 * (with its trailing slash, or empty), `prefix` the start of the entry after it. Undefined when the text doesn't end in one. */
export function atToken(text: string): { query: string; dir: string; prefix: string } | undefined {
  const query = /(?:^|\s)@([^\s@]*)$/.exec(text)?.[1];
  if (query === undefined) return undefined;
  const dir = query.slice(0, query.lastIndexOf("/") + 1);
  return { query: query.toLowerCase(), dir, prefix: query.slice(dir.length).toLowerCase() };
}

/** How the menu finds files for an `@` token's `query`: a name with no "/" in it searches the whole project
 * for it; nothing typed yet, a path, or just dots (on the way to `../`) browses folder by folder. */
export function mentionMode(query: string): "search" | "browse" {
  return query.includes("/") || /^\.*$/.test(query) ? "browse" : "search";
}

export type Entry = { name: string; dir: boolean };

/** The folder's entries starting with `prefix`; hidden ones only once a "." is typed. */
export function matchingEntries(entries: Entry[], prefix: string): Entry[] {
  return entries.filter((e) => e.name.toLowerCase().startsWith(prefix) && (prefix.startsWith(".") || !e.name.startsWith("."))).slice(0, 50);
}

/** The last search's results that `query` still matches: kept up while the search for `query` runs. The search
 * matches its query in order anywhere in a path, ignoring case, so a result for a shorter query still is one for
 * `query` only if `query` matches it the same way. Nothing when `query` doesn't extend the last search's `typed`. */
export function stillFound(found: { typed: string; entries: Entry[] }, query: string): Entry[] {
  if (!query.startsWith(found.typed)) return [];
  const q = [...query.toLowerCase()];
  return found.entries.filter((e) => {
    let n = 0;
    for (const c of e.name.toLowerCase()) if (c === q[n]) n++;
    return n >= q.length;
  });
}

/** `text` with its trailing `@` token replaced by the pick: an agent's name, or a path. A folder keeps
 * the token open on its contents, like tab completion in a shell; anything else ends it with a space. */
export function withPick(text: string, pick: { agent: string } | { dir: string; entry: Entry }): string {
  return text.replace(/@[^\s@]*$/, "agent" in pick ? `@${pick.agent} ` : `@${pick.dir}${pick.entry.name}${pick.entry.dir ? "/" : " "}`);
}
