import type { Agent } from "./api";

/** A Redraw click the agent hasn't confirmed yet, with the sticker it had then. */
export type RedrawClick = { avatar: string } | null;

type Redrawable = Pick<Agent, "redrawing" | "avatar">;

/** The click, as the agent's sticker is when the user clicks Redraw. */
export const redrawClick = (a: Redrawable): RedrawClick => ({ avatar: JSON.stringify(a.avatar) });

/** What's left of a click once the agent has moved on: nothing once it says it's redrawing (its `redrawing` flag takes
 * over from here) or its sticker has changed; otherwise the click, which keeps the button busy until the event arrives. */
export function pendingClick(click: RedrawClick, a: Redrawable): RedrawClick {
  if (!click || a.redrawing || JSON.stringify(a.avatar) !== click.avatar) return null;
  return click;
}

/** Whether the Redraw button is busy (disabled, with a spinner): a Redraw is queued or drawing, or was just clicked. */
export const redrawBusy = (a: Redrawable, click: RedrawClick) => !!a.redrawing || pendingClick(click, a) !== null;
