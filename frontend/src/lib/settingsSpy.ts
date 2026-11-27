/** Which Settings card "On this page" highlights: the last one whose top has passed `line`. At the bottom of the page the
 * last cards can't reach the line, so the card just clicked (`pinned`) counts, else the last one. */
export function cardInView(ids: readonly string[], tops: Record<string, number | undefined>, line: number, atBottom: boolean, pinned: string | null): string {
  const present = ids.filter((id) => tops[id] !== undefined);
  if (atBottom && present.length) return pinned && present.includes(pinned) ? pinned : present[present.length - 1];
  let current = ids[0];
  for (const id of present) if (tops[id]! <= line) current = id;
  return current;
}
