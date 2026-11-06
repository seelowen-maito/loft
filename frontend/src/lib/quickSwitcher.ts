/** The quick switcher: its key combination, the agents opened most recently and the search. All kept per browser.
 * `storage` defaults to localStorage, read inside the try since it can throw when site data is blocked. */

type Store = Pick<Storage, "getItem" | "setItem">;
type Keyed = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">;

/** ⌘ on a Mac, Ctrl elsewhere. */
export const DEFAULT_COMBO = "Mod+K";
/** Sent on `window` when Settings saves a new combination, so the switcher uses it at once. */
export const SWITCHER_KEY_CHANGED = "loft:switcher-key-changed";

const KEY = "loft:switcher-key";
const RECENT_KEY = "loft:recent-agents";
const RECENT_MAX = 20;
const MODIFIER_CODES = /^(Shift|Control|Alt|Meta|OS)(Left|Right)?$/;

export const isMac = () => typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** The pressed combination as `Ctrl+Alt+Shift+Meta+K`, or null for a lone modifier or a key with no Ctrl, Alt or ⌘:
 * plain keys and Shift+key are typing and never become a shortcut. The physical key is used, so ⌥K on a Mac is K, not ˚. */
export function comboFromEvent(e: Keyed): string | null {
  if (!e.ctrlKey && !e.altKey && !e.metaKey) return null;
  const code = e.code ?? "";
  if (MODIFIER_CODES.test(code) || MODIFIER_CODES.test(e.key)) return null;
  const key = code.match(/^Key([A-Z])$/)?.[1] ?? code.match(/^Digit(\d)$/)?.[1] ?? (code || (e.key.length === 1 ? e.key.toUpperCase() : e.key));
  if (!key) return null;
  return [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Meta", key].filter(Boolean).join("+");
}

/** `Mod` read as ⌘ on a Mac and Ctrl elsewhere. */
const resolve = (combo: string, mac: boolean) => combo.replace(/^Mod\+/, mac ? "Meta+" : "Ctrl+");

export function matchesCombo(e: Keyed, combo: string, mac = isMac()): boolean {
  return comboFromEvent(e) === resolve(combo, mac);
}

/** How the combination reads: `⌘K`, `⌃⇧P` on a Mac; `Ctrl+K` elsewhere. */
export function formatCombo(combo: string, mac = isMac()): string {
  const parts = resolve(combo, mac).split("+");
  const key = parts.pop()!;
  if (!mac) return [...parts, key].join("+");
  const sym: Record<string, string> = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Meta: "⌘" };
  return parts.map((p) => sym[p] ?? p).join("") + key;
}

/** The saved combination, or ⌘K / Ctrl+K when none is saved or the saved one has no modifier. */
export function loadCombo(storage?: Store): string {
  try {
    const v = (storage ?? localStorage).getItem(KEY);
    return v && /^(Mod|Ctrl|Alt|Meta)\+/.test(v) ? v : DEFAULT_COMBO;
  } catch {
    return DEFAULT_COMBO;
  }
}

export function saveCombo(combo: string, storage?: Store) {
  try {
    (storage ?? localStorage).setItem(KEY, combo);
  } catch {
    /* storage unavailable */
  }
}

export function loadRecent(storage?: Store): string[] {
  try {
    const v = JSON.parse((storage ?? localStorage).getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}

/** Puts `id` first among the agents opened recently. */
export function saveRecent(id: string, storage?: Store) {
  try {
    const list = [id, ...loadRecent(storage).filter((x) => x !== id)].slice(0, RECENT_MAX);
    (storage ?? localStorage).setItem(RECENT_KEY, JSON.stringify(list));
  } catch {
    /* storage unavailable */
  }
}

type Searchable = { id: string; name: string; title: string; group?: string; project?: string; projects?: string[] };

/** The switcher's list. Empty query: the agents opened most recently first, then the rest in their order.
 * Otherwise every word must appear in the name, title, group or a project; names that start with the query come first, then names containing it. */
export function switcherAgents<T extends Searchable>(agents: T[], query: string, recent: string[]): T[] {
  const q = query.trim().toLowerCase();
  if (!q) {
    const rank = (a: T) => (recent.includes(a.id) ? recent.indexOf(a.id) : recent.length);
    return agents.map((a, i) => ({ a, i })).sort((x, y) => rank(x.a) - rank(y.a) || x.i - y.i).map((x) => x.a);
  }
  const words = q.split(/\s+/);
  const score = (a: T) => {
    const name = a.name.toLowerCase();
    return name.startsWith(q) ? 0 : name.includes(q) ? 1 : 2;
  };
  return agents
    .filter((a) => {
      const text = [a.name, a.title, a.group ?? "", a.project ?? "", ...(a.projects ?? [])].join(" ").toLowerCase();
      return words.every((w) => text.includes(w));
    })
    .map((a, i) => ({ a, i }))
    .sort((x, y) => score(x.a) - score(y.a) || x.i - y.i)
    .map((x) => x.a);
}

/** True while Settings records a new combination, so pressing the current one doesn't open the switcher. */
export const recording = { on: false };
