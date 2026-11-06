/** Where a chart tooltip goes on screen: above its mark, inside the viewport. */

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * The tooltip's fixed `left`/`top` for a mark at `anchor` (screen coordinates).
 * Centered above the mark `gap` px away, flipped below it when there's no room
 * above, and kept `margin` px inside the viewport's edges.
 */
export function tipPosition(
  anchor: Box,
  tip: { width: number; height: number },
  viewport: { width: number; height: number },
  gap = 6,
  margin = 8,
): { left: number; top: number } {
  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)));
  const left = clamp(anchor.left + anchor.width / 2 - tip.width / 2, margin, viewport.width - margin - tip.width);
  let top = anchor.top - gap - tip.height;
  if (top < margin) top = anchor.top + anchor.height + gap;
  return { left, top: clamp(top, margin, viewport.height - margin - tip.height) };
}
