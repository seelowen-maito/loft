/** Needs you on the web: the list `GET /api/needs-you` returns (docs/needs-you-api.md), where each item leads, and the
 * top bar's counter. Item text is plain text: render it as text, never as HTML. */
import { NEEDS_YOU_LABEL } from "./taskStatus.ts";

/** Where an `open` action goes. */
export type NeedsYouTarget = { screen?: string; agent_id?: string; task_id?: string; url?: string };

export type NeedsYouAction = {
  kind: string;
  label: string;
  method?: string;
  path?: string;
  body?: Record<string, unknown>;
  target?: NeedsYouTarget;
};

export type NeedsYouItem = {
  id: string;
  type: string;
  urgency: number;
  agent: { id: string; name: string } | null;
  task_id: string | null;
  title: string;
  reason: string;
  text: string;
  created_at: string;
  age_seconds: number;
  links: { label: string; url?: string; path?: string }[];
  actions: NeedsYouAction[];
  project: string | null;
  /** Seen on some client (the list was opened there). Seen items stay listed, less prominent. */
  seen?: boolean;
};

/** `count`/`urgent`: the whole list. `new`/`new_urgent`: what the user hasn't seen, which the counter shows. */
export type NeedsYouList = { version: number; count: number; urgent: number; new: number; new_urgent: number; generated_at: string; items: NeedsYouItem[] };

/** A page of this web app (`to`), or another site (`href`). */
export type Place = { to: string } | { href: string };

/** A Loft page on this Mac (Settings → Push, Backups, Disk) opens here; anything else (GitHub, Screen Sharing) outside. */
function urlPlace(url: string, host: string): Place {
  try {
    const u = new URL(url);
    if ((u.protocol === "http:" || u.protocol === "https:") && u.hostname === host && !u.pathname.startsWith("/api")) return { to: `${u.pathname}${u.search}${u.hash}` };
  } catch {
    /* not a URL: open it as given */
  }
  return { href: url };
}

/** Where an `open` action's target leads: the agent's chat, the issue (with its Delivery card and Resume), this page, or a URL. */
export function placeOf(t: NeedsYouTarget | undefined, host: string): Place | null {
  if (!t) return null;
  if (t.screen === "chat" && t.agent_id) return { to: `/agents/${encodeURIComponent(t.agent_id)}` };
  if (t.screen === "task" && t.task_id) return { to: `/issues/${encodeURIComponent(t.task_id)}` };
  if (t.screen === "needs_you") return { to: "/needs-you" };
  if (t.url) return urlPlace(t.url, host);
  return null;
}

/** What an item card shows besides its text: the places to go (open actions, then links, each place once, never this
 * page), the requests it can send, its answer box and its Done check (the `dismiss` action: it takes the item out of
 * the list). */
export type ItemView = {
  open: { label: string; place: Place }[];
  calls: NeedsYouAction[];
  answer: NeedsYouAction | null;
  dismiss: NeedsYouAction | null;
};

export function itemView(i: NeedsYouItem, host: string): ItemView {
  const open: ItemView["open"] = [];
  const seen = new Set<string>();
  const add = (label: string, place: Place | null) => {
    const key = place && ("to" in place ? place.to : place.href);
    // This page itself (a Delivery with no issue to open) isn't a place to go from it.
    if (!place || !key || key === "/needs-you" || seen.has(key)) return;
    seen.add(key);
    open.push({ label, place });
  };
  for (const a of i.actions) if (a.kind === "open") add(a.label, placeOf(a.target, host));
  for (const l of i.links) add(l.label, l.path ? { to: l.path } : l.url ? urlPlace(l.url, host) : null);
  const http = (a: NeedsYouAction) => a.kind !== "open" && !!a.path && !!a.method;
  return {
    open,
    calls: i.actions.filter((a) => http(a) && a.kind !== "answer" && a.kind !== "dismiss"),
    answer: i.actions.find((a) => http(a) && a.kind === "answer") ?? null,
    dismiss: i.actions.find((a) => http(a) && a.kind === "dismiss") ?? null,
  };
}

/** The body an answer sends: the action's own, with the user's words as its `text`. */
export function answerBody(a: NeedsYouAction, text: string): Record<string, unknown> {
  return { ...(a.body ?? {}), text };
}

/** How urgent an item is, in words and colour. */
export function urgencyOf(u: number): { label: string; tone: "bad" | "warn" | "accent" | "muted" } {
  if (u >= 3) return { label: "Blocks an agent", tone: "bad" };
  if (u === 2) return { label: NEEDS_YOU_LABEL, tone: "warn" };
  if (u === 1) return { label: "Worth a look", tone: "accent" };
  return { label: "To review", tone: "muted" };
}

/** The counter's numbers: the items the user hasn't seen, and how many of those are urgent. */
export type Counts = { count: number; urgent: number };

/** The counter from the list: only what hasn't been seen. */
export function countsOf(l: Pick<NeedsYouList, "new" | "new_urgent">): Counts {
  return { count: l.new ?? 0, urgent: l.new_urgent ?? 0 };
}

/** The counter after a live event: a `needs_you` event carries the new unseen counts; any other leaves it. */
export function countAfter(c: Counts, e: { type: string; new?: number; new_urgent?: number }): Counts {
  if (e.type !== "needs_you" || typeof e.new !== "number") return c;
  return { count: e.new, urgent: e.new_urgent ?? 0 };
}

/** Amber only when something unseen is urgent. */
export function badgeTone(c: Counts): "warn" | "accent" {
  return c.urgent > 0 ? "warn" : "accent";
}

/** The counter's number: nothing at zero, "99+" past 99. */
export function badgeText(count: number): string {
  if (count <= 0) return "";
  return count > 99 ? "99+" : String(count);
}

export function badgeTitle(c: Counts): string {
  if (c.count <= 0) return "Nothing new needs you";
  const things = `${c.count} new ${c.count === 1 ? "thing needs" : "things need"} you`;
  return c.urgent > 0 ? `${things}, ${c.urgent} urgent` : things;
}

/** The items to mark seen: the ones shown that no client has seen yet. */
export function unseenIds(items: Pick<NeedsYouItem, "id" | "seen">[]): string[] {
  return items.filter((i) => !i.seen).map((i) => i.id);
}

/** Whether the user can be looking at the page: its tab is visible and its window has focus (a visible tab in a window
 * behind another app, or on another screen, isn't being read). */
export function beingLookedAt(d: { visibilityState: string; hasFocus(): boolean }): boolean {
  return d.visibilityState === "visible" && d.hasFocus();
}

/** The items shown as new during this visit: those unseen when they appeared here stay new until the page closes, even
 * once marked seen; ids that left the list drop out. */
export function freshAfter(fresh: Set<string>, items: Pick<NeedsYouItem, "id" | "seen">[]): Set<string> {
  const ids = new Set(items.map((i) => i.id));
  return new Set([...[...fresh].filter((id) => ids.has(id)), ...unseenIds(items)]);
}

/** The header's counts: how many are new, and how many in all. */
export function headline(total: number, fresh: number): string {
  if (total <= 0) return "";
  return fresh > 0 ? `${fresh} new · ${total} in all` : `${total} waiting, none new`;
}

/** What Mark all done does: the items that offer Done go (`ids`, what the call sends), the others stay. */
export function clearAll(items: NeedsYouItem[]): { ids: string[]; clear: number; stay: number } {
  const ids = items.filter((i) => i.actions.some((a) => a.kind === "dismiss")).map((i) => i.id);
  return { ids, clear: ids.length, stay: items.length - ids.length };
}

/** The confirm's title for the items Mark all done was opened with. */
export const clearTitle = (n: number) => `Mark ${n} ${n === 1 ? "item" : "items"} done?`;

/** The tag that opens an item: what kind of thing waits, in the `.chip` look with a tone's colour. */
export type KindTag = { label: string; tone: "bad" | "warn" | "info" | "good" };

export function kindTag(type: string): KindTag {
  switch (type) {
    case "task_blocked":
    case "task_failed":
    case "stuck":
    case "delivery_stalled":
      return { label: "Blocked", tone: "bad" };
    case "question":
    case "grill_round":
    case "comment_approval":
      return { label: "Question", tone: "warn" };
    case "permission":
    case "mac_access":
      return { label: "Permission", tone: "warn" };
    case "pr_workflows":
    case "actions_quota":
      return { label: "CI warning", tone: "warn" };
    case "pr_review":
      return { label: "Review", tone: "info" };
    case "pr_ready":
    case "task_done":
    case "delivery_done":
    case "delivery_merge":
      return { label: "Ready", tone: "good" };
    default:
      return { label: "Heads-up", tone: "warn" };
  }
}

/** A title as "api#1865: Multi experiences" for "dineouticeland/api#1865: Multi experiences"; `full` is the tooltip. The
 * project is left out of the meta line when the title already names it. */
export function shortTitle(title: string, project: string | null): { text: string; full: string; project: string | null } {
  const text = title.replace(/^[\w.-]+\/([\w.-]+#\d+)/, "$1");
  return { text, full: title, project: project && text.startsWith(`${project}#`) ? null : project };
}

/** Whether a `needs_you` event's ids differ from the items shown, so the page reloads. */
export function idsChanged(shown: string[], ids: string[]): boolean {
  return shown.length !== ids.length || shown.some((id, n) => id !== ids[n]);
}

/** Where closing Needs you leads: back to the page it was opened from, or home when this tab started on it (opened
 * by its URL: the router's first entry has the key "default"). */
export function closeTarget(historyKey: string): "back" | "/" {
  return historyKey === "default" ? "/" : "back";
}

/** What the top bar's icon does: open Needs you, or close it when it's already open. */
export function iconAction(pathname: string): "open" | "close" {
  return pathname === "/needs-you" ? "close" : "open";
}

/** Whether a key press closes Needs you: Esc, unless the user is typing (an answer), composing, a handler already used
 * it, or a dialog over the page (⌘K, ?) takes it. */
export function escCloses(e: { key: string; defaultPrevented: boolean; isComposing: boolean; typing: boolean; dialogOpen: boolean }): boolean {
  return e.key === "Escape" && !e.defaultPrevented && !e.isComposing && !e.typing && !e.dialogOpen;
}
