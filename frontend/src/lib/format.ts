import type { Agent, CleanItem, CleanLog, GitCheck, QuietWindow } from "./api";


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

/** Disk size in decimal units, like Finder: 980 KB, 4.2 MB, 12 GB. */
export function fmtBytes(n: number): string {
  const units = ["bytes", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1000 && i < units.length - 1) { n /= 1000; i++; }
  return i === 0 ? `${n} bytes` : `${n < 10 ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
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

/** Clean: a main clone itself (labelled, opens, at the top) can't be ticked, nor can a blocked item; folders inside one can. */
export function canTick(i: CleanItem, depth: number): boolean {
  return !i.blocked && !(i.opens && depth === 0 && !!i.label);
}

/** Clean: the `rm -rf` shown under an item; none when it can't be removed. */
export function shownCommand(i: CleanItem, depth: number): string {
  return canTick(i, depth) ? i.command : "";
}

/** `s` without control characters, so a name stays on its one line (Clean's `# <agent>` in Copy commands). */
export function oneLine(s: string): string {
  return s.replace(/[\u0000-\u001f\u007f-\u009f]/g, "");
}

/** Clean: whether a row gets a Check button (a clone or worktree at the top of a workspace). */
export function canCheck(i: CleanItem, depth: number): boolean {
  return depth === 0 && (i.kind === "clone" || i.kind === "worktree");
}

/** Clean: what a Check found, after its sign: "✅ Clean, all pushed: safe to delete" or "⚠️ 2 commits not pushed (main) · …". */
export function gitCheckText(c: GitCheck): string {
  return `${c.state === "safe" ? "✅" : c.state === "unsafe" ? "⚠️" : "❔"} ${c.lines.join(" · ")}`;
}

/** Clean: a log line, past tense like its header: "Removed /path (2.2 GB)" or "failed /path: why". */
export function cleanLineText(l: CleanLog["lines"][number]): string {
  if (l.done) return l.error ? `failed: ${l.done} ${l.rel}: ${l.error}` : `${l.done} ${l.rel}${l.bytes ? ` (${fmtBytes(l.bytes)})` : ""}`;
  return l.error ? `failed ${l.path}: ${l.error}` : `Removed ${l.path} (${fmtBytes(l.bytes)})`;
}

/** Clean's table: what the Status column says about an item ("Working", "Kept: main clone of loft", "Deletable"),
 * and whether it is locked (can't be ticked). */
export function cleanStatus(i: CleanItem, depth: number): { text: string; locked: boolean } {
  if (i.blocked) return { text: i.blocked === "Working" ? "Working" : `Locked: ${i.blocked}`, locked: true };
  if (!canTick(i, depth)) return { text: `Kept: ${i.label}`, locked: true };
  return { text: "Deletable", locked: false };
}

/** Clean's Status once git is known: a clone or worktree is "Deletable · checked" only after a clean Check, and says
 * "Check git first" until then. Locked items and plain folders keep `cleanStatus`. */
export function cleanState(i: CleanItem, depth: number, check?: GitCheck | null): { text: string; locked: boolean } {
  const s = cleanStatus(i, depth);
  if (s.locked || !canCheck(i, depth)) return s;
  return { text: check?.state === "safe" ? "Deletable · checked" : "Check git first", locked: false };
}

/** Clean's select all: the visible rows that may be ticked (not locked, and a clone only once checked safe). */
export function selectable(items: CleanItem[], checks: Map<string, GitCheck | null>): CleanItem[] {
  return items.filter((i) => !cleanStatus(i, 0).locked && (!canCheck(i, 0) || checks.get(i.path)?.state === "safe"));
}

/** Clean's starting ticks: what the API marks ticked (test output, closed scratch) and the table may tick. A clone
 * isn't, whatever the API says, until a Check git finds it safe. */
export function initialTicks(items: CleanItem[]): Set<string> {
  return new Set(items.filter((i) => i.ticked && !cleanStatus(i, 0).locked && !canCheck(i, 0)).map((i) => i.path));
}

/** Whether a finished Check leaves the row ticked: only "safe" does, so a re-check that finds unpushed work, or can't
 * tell, unticks it. */
export function ticksAfter(c: GitCheck): boolean {
  return c.state === "safe";
}

/** What Delete selected removes: the ticked rows the table shows, and the folders opened inside them. A row the
 * filters hide is never removed unseen, even if a late Check ticked it. */
export function chosenRows(shown: CleanItem[], inside: Map<string, CleanItem[] | null>, ticked: Set<string>): CleanItem[] {
  return shown.flatMap((i) => [i, ...(inside.get(i.path) ?? [])]).filter((i) => ticked.has(i.path));
}

/** Runs `fn` over `xs`, at most `limit` at a time; `onDone(n)` after each finishes. `fn` must not throw. */
export async function inBatches<T>(xs: T[], limit: number, fn: (x: T) => Promise<void>, onDone: (n: number) => void = () => {}): Promise<void> {
  let next = 0;
  let done = 0;
  const worker = async () => {
    while (next < xs.length) {
      await fn(xs[next++]);
      onDone(++done);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, xs.length) }, worker));
}

/** Clean's Type column: a clone says "git clone", build output says "build output". */
export function cleanType(i: CleanItem): string {
  return i.kind === "clone" ? "git clone" : i.kind === "build" ? "build output" : i.kind;
}

export type CleanSort = { by: "size" | "agent" | "changed"; desc: boolean };

/** Clean's table order: by size, agent name or last change; ties go biggest first, then by name. */
export function sortClean(items: CleanItem[], sort: CleanSort, agentName: (id: string) => string): CleanItem[] {
  const key = (i: CleanItem) => (sort.by === "size" ? i.bytes : sort.by === "changed" ? new Date(i.changed).getTime() || 0 : agentName(i.agent).toLowerCase());
  const dir = sort.desc ? -1 : 1;
  return [...items].sort((a, b) => {
    const ka = key(a);
    const kb = key(b);
    return (ka < kb ? -dir : ka > kb ? dir : 0) || b.bytes - a.bytes || a.name.localeCompare(b.name);
  });
}

/** A locked reason is the git check's lines joined with " · ": the first one, and how many more there are. */
export function reasonLines(blocked: string): string[] {
  return blocked.split(" · ").filter(Boolean);
}

export type CleanFilter = { agent: string; type: string; deletable: boolean };

/** The Clean table's filters: one agent, one type ("" is all), and only what can be ticked. */
export function filterClean(items: CleanItem[], f: CleanFilter): CleanItem[] {
  return items.filter((i) => (!f.agent || i.agent === f.agent) && (!f.type || cleanType(i) === f.type) && (!f.deletable || !cleanStatus(i, 0).locked));
}
