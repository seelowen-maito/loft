/** What a key does in the Projects folder's suggestion list of `count` rows, `index` highlighted: move the highlight,
 * pick the highlighted folder, close the list, or nothing (the key goes to the field). */
export type FolderKey = { move: number } | { pick: number } | "close" | undefined;

export function folderKey(key: string, index: number, count: number, shift = false): FolderKey {
  if (count === 0) return undefined;
  if (key === "ArrowDown" || key === "ArrowUp") return { move: (index + (key === "ArrowDown" ? 1 : count - 1)) % count };
  if ((key === "Tab" || key === "Enter") && !shift) return { pick: Math.min(index, count - 1) };
  if (key === "Escape") return "close";
  return undefined;
}

/** "1 repo", "12 repos", "no repos". */
export const repoCount = (n: number) => (n === 0 ? "no repos" : `${n} repo${n === 1 ? "" : "s"}`);

/** The suggestions answered for `asked` that still fit `typed` while its own answer comes: the ones whose folder name
 * starts with what's typed after the last `/`, none once a `/` begins a new folder. The typed path itself isn't one. */
export function stillMatching<T extends { path: string }>(asked: string, typed: string, list: T[]): T[] {
  const fresh = asked === typed;
  const part = typed.split("/").pop()!.toLowerCase();
  if (!fresh && part === "") return [];
  return list.filter((f) => f.path !== typed && (fresh || f.path.split("/").pop()!.toLowerCase().startsWith(part)));
}
