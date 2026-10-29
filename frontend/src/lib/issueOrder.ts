/** How the Issues list orders and annotates rows: what an agent can pick up first, and a spec's progress. */
type Row = { id: string; github?: { sub_total?: number; sub_done?: number } | null; labels: string[]; priority: "low" | "normal" | "high" | "urgent"; updated_at: string; parent_id: string | null; status: string };

const PRIORITY_RANK = { urgent: 0, high: 1, normal: 2, low: 3 } as const;

export const isSpec = (t: { labels: string[] }) => t.labels.includes("spec");
const ready = (t: { labels: string[] }) => t.labels.includes("ready-for-agent");

/** Open rows: `ready-for-agent` first, then by priority, then the most recently changed. */
export function compareOpen(a: Row, b: Row): number {
  return Number(ready(b)) - Number(ready(a)) || PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority] || b.updated_at.localeCompare(a.updated_at);
}

/** A spec's tickets done, of all its tickets; null for an issue that isn't a spec or has none. GitHub's own count wins (it includes closed sub-issues Loft doesn't mirror); a Loft-only spec counts its Loft sub-issues. */
export function specProgress(spec: Row, all: Row[]): { done: number; total: number } | null {
  if (!isSpec(spec)) return null;
  if (spec.github?.sub_total) return { done: spec.github.sub_done ?? 0, total: spec.github.sub_total };
  const kids = all.filter((t) => t.parent_id === spec.id && t.status !== "cancelled");
  return kids.length ? { done: kids.filter((t) => t.status === "done").length, total: kids.length } : null;
}
