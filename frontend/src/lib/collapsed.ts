/** Project groups folded away in a list, remembered per browser under `key`; projects not in it are expanded.
 * `storage` defaults to localStorage, read inside the try since it can throw when site data is blocked. */

type Store = Pick<Storage, "getItem" | "setItem">;

export function loadCollapsed(key: string, storage?: Store): string[] {
  try {
    const v = JSON.parse((storage ?? localStorage).getItem(key) ?? "[]");
    return Array.isArray(v) ? v.filter((p): p is string => typeof p === "string") : [];
  } catch {
    return [];
  }
}

/** Folds `project` if it is open, opens it if folded, and saves the result. */
export function toggleCollapsed(key: string, collapsed: string[], project: string, storage?: Store): string[] {
  const next = collapsed.includes(project) ? collapsed.filter((p) => p !== project) : [...collapsed, project];
  try {
    (storage ?? localStorage).setItem(key, JSON.stringify(next));
  } catch {
    /* storage unavailable */
  }
  return next;
}
