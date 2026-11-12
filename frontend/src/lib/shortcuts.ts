/** The web app's keyboard shortcuts: which key does what, the two rebindable combinations, the agents opened most recently
 * and the quick switcher's search. All kept per browser.
 * `storage` defaults to localStorage, read inside the try since it can throw when site data is blocked. */

import type { PullRequest, Task } from "./api";

type Store = Pick<Storage, "getItem" | "setItem">;
type Keyed = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">;

/** The shortcuts that can be rebound in Settings. `Mod` is ⌘ on a Mac and Ctrl elsewhere. */
export const REBINDABLE = [
  { id: "switcher", label: "Search agents, specs, issues and PRs", default: "Mod+K" },
  { id: "next", label: "Next agent that needs you", default: "Mod+J" },
] as const;
export type Rebindable = (typeof REBINDABLE)[number]["id"];
export type Combos = Record<Rebindable, string>;

/** `g` then a letter. */
export const GO: Record<string, { path: string; label: string }> = {
  a: { path: "/agents", label: "Agents" },
  i: { path: "/issues", label: "Issues" },
  r: { path: "/repos", label: "Repos" },
  s: { path: "/settings", label: "Settings" },
  m: { path: "/simulator", label: "Simulator" },
};

/** The fixed shortcuts, for the sheet and Settings. */
export const FIXED: { keys: string; label: string }[] = [
  ...Object.entries(GO).map(([k, g]) => ({ keys: `g ${k}`, label: `Go to ${g.label}` })),
  { keys: "/", label: "Focus the chat box" },
  { keys: "?", label: "Show shortcuts" },
];

/** Sent on `window` when Settings saves a combination, so the shortcuts use it at once. */
export const SHORTCUTS_CHANGED = "loft:shortcuts-changed";

/** Sent on `window` to open the search or the shortcut sheet from a click, e.g. the chat's hint line. */
export const OPEN_SHORTCUT = "loft:open-shortcut";
export type Openable = "switcher" | "sheet";

export function openShortcut(which: Openable, target: EventTarget = window) {
  target.dispatchEvent(new CustomEvent<Openable>(OPEN_SHORTCUT, { detail: which }));
}

const keyOf = (id: Rebindable) => `loft:shortcut:${id}`;
const RECENT_KEY = "loft:recent-agents";
const RECENT_MAX = 20;
const MODIFIER_CODES = /^(Shift|Control|Alt|Meta|OS)(Left|Right)?$/;

export const isMac = () => typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** The pressed combination as `Ctrl+Alt+Shift+Meta+K`, or null for a lone modifier or a key with no Ctrl, Alt or ⌘:
 * plain keys and Shift+key are typing and never become a combination. The physical key is used, so ⌥K on a Mac is K, not ˚. */
export function comboFromEvent(e: Keyed): string | null {
  if (!e.ctrlKey && !e.altKey && !e.metaKey) return null;
  const code = e.code ?? "";
  if (MODIFIER_CODES.test(code) || MODIFIER_CODES.test(e.key)) return null;
  const key = code.match(/^Key([A-Z])$/)?.[1] ?? code.match(/^Digit(\d)$/)?.[1] ?? (code || (e.key.length === 1 ? e.key.toUpperCase() : e.key));
  if (!key) return null;
  return [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Meta", key].filter(Boolean).join("+");
}

const resolve = (combo: string, mac: boolean) => combo.replace(/^Mod\+/, mac ? "Meta+" : "Ctrl+");

/** A recorded combination as stored: ⌘K on a Mac and Ctrl+K elsewhere become `Mod+K`, so it equals the default when it is one. */
export function normalizeCombo(combo: string, mac = isMac()): string {
  return combo.replace(mac ? /^Meta\+(?=[^+]+$)/ : /^Ctrl\+(?=[^+]+$)/, "Mod+");
}

/** Copy, paste, select all, undo, close tab, reload and the like: taking them would break them on every page. */
export function isReserved(combo: string, mac = isMac()): boolean {
  const c = resolve(combo, mac);
  const mod = mac ? "Meta" : "Ctrl";
  return [..."CVXAZYWTNQRLFPS"].some((k) => c === `${mod}+${k}`) || [..."ZTN"].some((k) => c === (mac ? `Shift+Meta+${k}` : `Ctrl+Shift+${k}`));
}

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

const defaultOf = (id: Rebindable) => REBINDABLE.find((r) => r.id === id)!.default;

/** The saved combination, or the default when none is saved, the saved one has no modifier or is reserved for the browser. */
export function loadCombo(id: Rebindable, storage?: Store): string {
  try {
    const v = (storage ?? localStorage).getItem(keyOf(id));
    return v && /(^|\+)(Mod|Ctrl|Alt|Meta)\+/.test(v) && !isReserved(v) ? v : defaultOf(id);
  } catch {
    return defaultOf(id);
  }
}

export function loadCombos(storage?: Store): Combos {
  return { switcher: loadCombo("switcher", storage), next: loadCombo("next", storage) };
}

export function saveCombo(id: Rebindable, combo: string, storage?: Store) {
  try {
    (storage ?? localStorage).setItem(keyOf(id), combo);
  } catch {
    /* storage unavailable */
  }
}

/** Whether the key goes to a text field: then no plain-key shortcut fires. */
export function isTyping(target: unknown): boolean {
  const el = target as { tagName?: string; type?: string; isContentEditable?: boolean } | null;
  if (!el?.tagName) return false;
  if (el.isContentEditable || el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  return el.tagName === "INPUT" && !/^(checkbox|radio|button|submit|reset|range|color|file)$/.test(el.type ?? "");
}

export type Action =
  | { type: "switcher" }
  | { type: "next" }
  | { type: "go"; path: string }
  | { type: "g" }
  | { type: "focus-chat" }
  | { type: "sheet" };

/** What a key press does. The rebindable combinations work everywhere, even while typing; the plain keys only outside a text field.
 * `afterG` is true when the previous key was a lone `g`: the next letter picks the page, any other key cancels it. */
export function shortcutAction(e: Keyed, o: { combos: Combos; typing: boolean; afterG: boolean; mac?: boolean }): Action | null {
  const mac = o.mac ?? isMac();
  if (matchesCombo(e, o.combos.switcher, mac)) return { type: "switcher" };
  if (matchesCombo(e, o.combos.next, mac)) return { type: "next" };
  if (e.ctrlKey || e.metaKey || e.altKey || o.typing) return null;
  if (o.afterG) {
    const go = GO[e.key.toLowerCase()];
    return go ? { type: "go", path: go.path } : null;
  }
  if (e.key === "g") return { type: "g" };
  if (e.key === "/") return { type: "focus-chat" };
  if (e.key === "?") return { type: "sheet" };
  return null;
}

/** The agent ⌘J opens: the one after `current` among the agents in Needs you, in its order, wrapping; the first if `current` isn't there. */
export function nextNeedsYou(items: { agent: { id: string } | null }[], current: string | null): string | null {
  const ids = [...new Set(items.flatMap((i) => (i.agent ? [i.agent.id] : [])))];
  if (!ids.length) return null;
  const at = current ? ids.indexOf(current) : -1;
  return ids[(at + 1) % ids.length];
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

type Searchable = { id: string; name: string; group?: string; project?: string; projects?: string[] };

/** The switcher's list. Empty query: the agents opened most recently first, then the rest in their order.
 * Otherwise every word must appear in the name, Role label (`label`), group or a project; names that start with the query come first, then names containing it. */
export function switcherAgents<T extends Searchable>(agents: T[], query: string, recent: string[], label: (a: T) => string): T[] {
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
      const text = [a.name, label(a), a.group ?? "", a.project ?? "", ...(a.projects ?? [])].join(" ").toLowerCase();
      return words.every((w) => text.includes(w));
    })
    .map((a, i) => ({ a, i }))
    .sort((x, y) => score(x.a) - score(y.a) || x.i - y.i)
    .map((x) => x.a);
}

/** A spec, issue or pull request as the switcher searches it: `number` is null for a Loft-only issue. */
export type FoundRow = { title: string; number: number | null; open: boolean; project: string };

/** The switcher's best `limit` matches among specs, issues or pull requests. `#123` or `123` finds that number.
 * Otherwise every word must appear in the title or project; open rows come first, then titles that start with the query, then titles containing it. */
export function searchRows<T extends FoundRow>(rows: T[], query: string, limit = 5): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const num = q.match(/^#?(\d+)$/)?.[1];
  if (num) return rows.filter((r) => r.number === Number(num)).sort((a, b) => Number(b.open) - Number(a.open)).slice(0, limit);
  const words = q.split(/\s+/);
  const score = (r: T) => {
    const title = r.title.toLowerCase();
    return (r.open ? 0 : 3) + (title.startsWith(q) ? 0 : title.includes(q) ? 1 : 2);
  };
  return rows
    .filter((r) => words.every((w) => `${r.title} ${r.project}`.toLowerCase().includes(w)))
    .map((r, i) => ({ r, i }))
    .sort((x, y) => score(x.r) - score(y.r) || x.i - y.i)
    .map((x) => x.r)
    .slice(0, limit);
}

// ---- # tags in the chat ----
/** The `#` token typed at the end of the text: `#18` or `#`. What's typed after the `#`, else undefined. */
export const hashToken = (text: string) => /(?:^|[\s([{"'])#([^\s#]*)$/.exec(text)?.[1];

export type TagRow = { kind: "spec" | "issue" | "pr"; title: string; tag: string; open: boolean; project: string; number: number | null };

/** The tag to insert for a pick, as the backend reads it: `#N` in the agent's own project, `owner/repo#N` in another,
 * the issue's id for a Loft-only issue. */
export function tagRows(tasks: Task[], prs: PullRequest[], project: string): TagRow[] {
  const mine = project.trim();
  const fmt = (repo: string, number: number, name: string) => (mine && name === mine ? `#${number}` : `${repo}#${number}`);
  const issues = tasks.map((t): TagRow => ({
    kind: t.labels.some((l) => l.toLowerCase() === "spec") ? "spec" : "issue", title: t.title, project: t.project, number: t.github?.number ?? null, open: t.state === "open",
    tag: t.github ? fmt(t.github.repo, t.github.number, t.project) : `#${t.id}`,
  }));
  // The same PR can be registered by more than one agent.
  const unique = [...new Map(prs.map((p) => [p.url, p])).values()];
  const pulls = unique.map((p): TagRow => {
    const name = p.slug.split("/").pop() ?? p.slug;
    return { kind: "pr", title: p.title, project: name, number: p.number, open: p.state === "open" || p.state === "", tag: fmt(p.slug, p.number, name) };
  });
  return [...issues, ...pulls];
}

/** The picker's rows for what's typed after `#`, in the ⌘K search's order: specs, issues, then pull requests.
 * Nothing typed yet lists a few open ones of each. */
export function tagMatches(rows: TagRow[], query: string): TagRow[] {
  const per = query.trim() ? 4 : 3;
  return (["spec", "issue", "pr"] as const).flatMap((k) => {
    const of = rows.filter((r) => r.kind === k);
    return query.trim() ? searchRows(of, query, per) : of.filter((r) => r.open).slice(0, per);
  });
}

/** `text` with its trailing `#…` token replaced by the pick's tag and a space. */
export const insertTag = (text: string, tag: string) => text.replace(/#[^\s#]*$/, `${tag} `);

/** True while Settings records a combination, so no shortcut fires meanwhile. */
export const recording = { on: false };
