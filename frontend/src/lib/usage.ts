/** Reset and spend texts for the navbar usage meter's tooltips and the home line. */
import { fmtDayTime, relTime } from "./time.ts";

/** "Resets in 2h · Tue 2:00 PM" (the browser's hour cycle), or "Reset time unknown". */
export function resetText(resetsAt: string | null | undefined, now: number, locale?: string): string {
  const t = resetsAt ? new Date(resetsAt) : null;
  if (!t || Number.isNaN(t.getTime())) return "Reset time unknown";
  // A reset is ahead: one due or already past (usage not refreshed yet) is "in a moment", never "just now" or "ago".
  const when = t.getTime() - now < 45_000 ? "in a moment" : relTime(t.getTime() / 1000, now);
  return `Resets ${when} · ${fmtDayTime(t, now, locale)}`;
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
