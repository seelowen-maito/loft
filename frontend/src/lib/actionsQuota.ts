/** An owner's GitHub Actions minutes against its quota, as Settings and a project's page show them. */

/** How far `minutes` are into `quota`, in percent; none without a quota. */
export function quotaPercent(minutes: number, quota: number | null): number | null {
  return quota && quota > 0 ? (minutes / quota) * 100 : null;
}

/** The bar's color: past the 90% line bad, past 70% a warning, else good, like the Needs you warnings. */
export function quotaTone(percent: number): "bg-bad" | "bg-warn" | "bg-good" {
  return percent >= 90 ? "bg-bad" : percent >= 70 ? "bg-warn" : "bg-good";
}

/** A quota field's text as the API takes it: empty is none, else a whole number of minutes up to 1,000,000 (0 is
 *  none too). `undefined` when it's neither, so the field goes back to the saved quota. */
export function parseQuota(text: string): number | null | undefined {
  const t = text.trim().replace(/[,_\s]/g, "");
  if (t === "") return null;
  if (!/^\d+$/.test(t)) return undefined;
  const n = Number(t);
  if (n > 1_000_000) return undefined;
  return n === 0 ? null : n;
}

/** "November 2026" for `2026-11`. */
export function monthName(month: string): string {
  const [y, m] = month.split("-").map(Number);
  if (!y || !m) return month;
  return new Date(Date.UTC(y, m - 1, 1)).toLocaleString("en-US", { month: "long", year: "numeric", timeZone: "UTC" });
}
