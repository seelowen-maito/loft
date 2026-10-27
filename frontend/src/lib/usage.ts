/** Reset text for the navbar usage meter's tooltips. */

/** "in 2h", "in 3d": how long until `t`, rounded like timeAgo. */
function untilText(ms: number): string {
  const a = Math.max(0, Math.round(ms / 1000));
  if (a < 45) return "in a moment";
  if (a < 3600) return `in ${Math.round(a / 60)}m`;
  if (a < 86400) return `in ${Math.round(a / 3600)}h`;
  return `in ${Math.round(a / 86400)}d`;
}

/** "Resets in 2h · Tue 14:00" (24h clock, the user's locale), or "Reset time unknown". */
export function resetText(resetsAt: string | null | undefined, now: number, locale?: string): string {
  const t = resetsAt ? new Date(resetsAt) : null;
  if (!t || Number.isNaN(t.getTime())) return "Reset time unknown";
  const day = t.toLocaleDateString(locale, { weekday: "short" });
  const time = t.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit", hour12: false });
  return `Resets ${untilText(t.getTime() - now)} · ${day} ${time}`;
}
