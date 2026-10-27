/** The Repos page's agents section: cards ordered by activity, one grid row at a time. */

type Activity = {
  agent: { running?: boolean; last_run_at: string | null; created_at: string };
  last_reply: { at: string } | null;
};

/** When the agent was last active: now while it runs, else its latest run or reply, else when it was created. */
export function lastActivity(s: Activity, now = Date.now()): number {
  if (s.agent.running) return now;
  const times = [s.agent.last_run_at, s.last_reply?.at, s.agent.created_at].map((t) => (t ? Date.parse(t) : NaN));
  return Math.max(0, ...times.filter((t) => !Number.isNaN(t)));
}

/** Most recently active first; ties keep their order. */
export function byActivity<T extends Activity>(list: T[], now = Date.now()): T[] {
  return list
    .map((s, i) => ({ s, i, t: lastActivity(s, now) }))
    .sort((a, b) => b.t - a.t || a.i - b.i)
    .map((x) => x.s);
}

/** Columns of the grid at this width, matching its classes: 1, `sm:grid-cols-2` (40rem), `xl:grid-cols-3` (80rem). */
export function cardsPerRow(widthPx: number): number {
  return widthPx >= 1280 ? 3 : widthPx >= 640 ? 2 : 1;
}
