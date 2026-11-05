/** Reset and spend texts for the navbar usage meter's tooltips and the home line. */

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
  return `Resets ${untilText(t.getTime() - now)} · ${dayTime(t, locale)}`;
}

/** "Thu 04:00": short weekday and 24h time in the user's locale. */
function dayTime(t: Date, locale?: string): string {
  const day = t.toLocaleDateString(locale, { weekday: "short" });
  const time = t.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit", hour12: false });
  return `${day} ${time}`;
}

/** "$183": whole dollars with thousands separators, for the home line's cost. */
export function wholeDollars(usd: number): string {
  return `$${Math.round(usd).toLocaleString("en-US")}`;
}

/** "$1,438.75": dollars and cents with thousands separators. */
export function exactDollars(usd: number): string {
  return `$${usd.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/** The home line's change tag, this period's cost against the previous one's: "▲12%" (up, warn),
 *  "▼8%" (down, good) or "same" (muted). Null when the previous period cost nothing. */
export function costChange(cost: number, previous: number): { text: string; tone: "good" | "warn" | "muted" } | null {
  if (!(previous > 0)) return null;
  const pct = Math.round(((cost - previous) / previous) * 100);
  if (pct === 0) return { text: "same", tone: "muted" };
  return pct > 0 ? { text: `▲${pct}%`, tone: "warn" } : { text: `▼${-pct}%`, tone: "good" };
}
