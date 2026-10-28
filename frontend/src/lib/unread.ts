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
