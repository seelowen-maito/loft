/** Unread replies: an agent answered a run the user started and they haven't seen its chat since. */

type Unread = { id: string; unread_reply?: boolean };

/** Mark the open chat read: only while the browser tab is visible, so a reply that lands in a background tab stays unread. */
export function shouldMarkRead(agent: Unread | null, openId: string | null, visible: boolean): boolean {
  return !!agent?.unread_reply && agent.id === openId && visible;
}

/** The roster's dot by the name; never for the open chat, which is read once the tab is visible. */
export function showsUnreadDot(agent: Unread, openId: string | null): boolean {
  return !!agent.unread_reply && agent.id !== openId;
}

/** How many agents have an Unread reply, the open chat included: a reply that lands in a background tab counts until it's seen. */
export function unreadCount(agents: Unread[]): number {
  return agents.filter((a) => a.unread_reply).length;
}

/** The Agents tab dot's label: the count, capped at "9+" so the dot stays small. */
export function unreadBadge(count: number): string {
  return count > 9 ? "9+" : String(count);
}

/** The browser tab title: "(N) Loft" while there are Unread replies, "Loft" otherwise. */
export function unreadTitle(count: number): string {
  return count > 0 ? `(${count}) Loft` : "Loft";
}

/** A loaded agent list, with the live updates that arrived while it loaded laid over it (null: deleted meanwhile), as they're newer. */
export function mergeLoaded<T extends { id: string }>(loaded: T[], fresh: Map<string, T | null>): T[] {
  const kept = loaded.flatMap((a) => {
    if (!fresh.has(a.id)) return [a];
    const f = fresh.get(a.id);
    return f ? [{ ...a, ...f }] : [];
  });
  const known = new Set(loaded.map((a) => a.id));
  const added = [...fresh].flatMap(([id, a]) => (a && !known.has(id) ? [a] : []));
  return [...kept, ...added];
}
