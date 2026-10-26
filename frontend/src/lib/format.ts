import type { Agent, QuietWindow } from "./api";


export function timeAgo(input: string | number | null | undefined): string {
  if (input === null || input === undefined || input === "") return "never";
  const t = typeof input === "number" ? input * 1000 : new Date(input).getTime();
  const s = Math.round((Date.now() - t) / 1000);
  const future = s < 0;
  const a = Math.abs(s);
  const unit =
    a < 45 ? "just now"
    : a < 3600 ? `${Math.round(a / 60)}m`
    : a < 86400 ? `${Math.round(a / 3600)}h`
    : a < 86400 * 30 ? `${Math.round(a / 86400)}d`
    : a < 86400 * 365 ? `${Math.round(a / (86400 * 30))}mo`
    : `${Math.round(a / (86400 * 365))}y`;
  if (unit === "just now") return unit;
  return future ? `in ${unit}` : `${unit} ago`;
}

export function fmtNum(n: number): string {
  if (n >= 10000) return `${(n / 1000).toFixed(0)}k`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

export function fmtInterval(min: number): string {
  if (!min) return "manual";
  if (min % 1440 === 0) return `every ${min / 1440}d`;
  if (min % 60 === 0) return `every ${min / 60}h`;
  return `every ${min}m`;
}

export const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** "Mon–Fri", "Sat, Sun", "Mon, Wed–Fri". */
export function fmtDays(days: number[]): string {
  const parts: string[] = [];
  for (let i = 0; i < days.length; ) {
    let j = i;
    while (j + 1 < days.length && days[j + 1] === days[j] + 1) j++;
    parts.push(j === i ? DAYS[days[i]] : j === i + 1 ? `${DAYS[days[i]]}, ${DAYS[days[j]]}` : `${DAYS[days[i]]}–${DAYS[days[j]]}`);
    i = j + 1;
  }
  return parts.join(", ");
}

/** "Mon–Fri 09:00–17:00; Sat, Sun all day". */
export function fmtQuiet(windows: QuietWindow[]): string {
  return windows.map((w) => `${fmtDays(w.days)} ${w.start === w.end ? "all day" : `${w.start}–${w.end}`}`).join("; ");
}

/** The interval, marked custom when quiet hours shape it. */
export function fmtSchedule(a: Pick<Agent, "interval_minutes" | "quiet_hours">): string {
  return a.interval_minutes && a.quiet_hours?.length ? `${fmtInterval(a.interval_minutes)} · custom` : fmtInterval(a.interval_minutes);
}

/** Agents grouped by project, named groups A–Z, then those without one. */
export function groupByProject<T extends Pick<Agent, "project">>(agents: T[]): { project: string; agents: T[] }[] {
  const m = new Map<string, T[]>();
  agents.forEach((a) => {
    const p = a.project?.trim() ?? "";
    m.set(p, [...(m.get(p) ?? []), a]);
  });
  return [...m.entries()]
    .sort(([a], [b]) => (!a ? 1 : !b ? -1 : a.localeCompare(b, undefined, { sensitivity: "base" })))
    .map(([project, agents]) => ({ project, agents }));
}

const toMin = (t: string) => Number(t.slice(0, 2)) * 60 + Number(t.slice(3, 5));

/** Whether `minute` (since midnight) on `day` falls in a window. Mirrors backend/src/schedule.rs. */
export function isQuiet(windows: QuietWindow[], day: number, minute: number): boolean {
  return windows.some((w) => {
    const s = toMin(w.start), e = toMin(w.end);
    if (e > s) return w.days.includes(day) && minute >= s && minute < e;
    // Past midnight (or all day): the rest of its start day, then the next morning.
    return (w.days.includes(day) && minute >= s) || (w.days.includes((day + 6) % 7) && minute < e);
  });
}

export function cx(...c: (string | false | null | undefined)[]) {
  return c.filter(Boolean).join(" ");
}
