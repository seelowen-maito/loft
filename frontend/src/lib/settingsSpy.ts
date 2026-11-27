/** Which Settings card "On this page" highlights: the last one whose top has passed `line`. At the bottom of the page the
 * last cards can't reach the line, so the card just clicked (`pinned`) counts, else the last one. */
export function cardInView(ids: readonly string[], tops: Record<string, number | undefined>, line: number, atBottom: boolean, pinned: string | null): string {
  const present = ids.filter((id) => tops[id] !== undefined);
  if (atBottom && present.length) return pinned && present.includes(pinned) ? pinned : present[present.length - 1];
  let current = ids[0];
  for (const id of present) if (tops[id]! <= line) current = id;
  return current;
}

/** A card the user clicked (`top`: where the scroller was then) or the URL's #hash named (`top` null: the page is still
 * scrolling to it as the cards above grow). */
export type Pin = { id: string; top: number | null };

/** The pin after a scroll: a clicked card's is dropped once the scroller has moved from where the click left it. A
 * #hash pin only goes when the user scrolls by hand. */
export function keepPin(pin: Pin | null, scrollTop: number): Pin | null {
  return pin && pin.top !== null && Math.abs(scrollTop - pin.top) > 2 ? null : pin;
}
