// The one place times, dates and durations are worded. Clock times follow the browser's locale
// (so its 12h or 24h cycle); `locale` is only passed by tests.

type At = string | number | Date;

const ms = (at: At) => (typeof at === "number" ? at * 1000 : new Date(at).getTime());

/** "5m ago", "in 2h", "just now" (under 45 s either way), "never" for no time. Numbers are epoch seconds. */
export function relTime(input: At | null | undefined, now = Date.now()): string {
  if (input === null || input === undefined || input === "") return "never";
  const s = Math.round((now - ms(input)) / 1000);
  const a = Math.abs(s);
  const unit =
    a < 45 ? ""
    : a < 3600 ? `${Math.round(a / 60)}m`
    : a < 86400 ? `${Math.round(a / 3600)}h`
    : a < 86400 * 30 ? `${Math.round(a / 86400)}d`
    : a < 86400 * 365 ? `${Math.round(a / (86400 * 30))}mo`
    : `${Math.round(a / (86400 * 365))}y`;
  return !unit ? "just now" : s < 0 ? `in ${unit}` : `${unit} ago`;
}

/** When a check-in is next: "next in 5m", or "next: any moment" once it is due, overdue or under 45 s away. */
export function nextRunText(at: At, now = Date.now()): string {
  return ms(at) - now < 45_000 ? "next: any moment" : `next ${relTime(at, now)}`;
}

/** A span of time: "45s", "2m 23s", "1h 4m", "2d 3h". Negative spans (clock skew) read as "0s". */
export function fmtDuration(span: number): string {
  const s = Math.max(0, Math.floor(span / 1000));
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

/** "14:05" or "2:05 PM"; with `seconds`, "14:05:09". */
export function fmtClock(at: At, opts: { seconds?: boolean; locale?: string } = {}): string {
  return new Date(ms(at)).toLocaleTimeString(opts.locale, { hour: "numeric", minute: "2-digit", ...(opts.seconds ? { second: "2-digit" } : {}) });
}

/** "Nov 15, 2026". */
export function fmtDate(at: At, locale?: string): string {
  return new Date(ms(at)).toLocaleDateString(locale, { dateStyle: "medium" });
}

/** "Nov 15, 2026, 2:05 PM". */
export function fmtDateTime(at: At, locale?: string): string {
  return new Date(ms(at)).toLocaleString(locale, { dateStyle: "medium", timeStyle: "short" });
}

/** "Tue 14:00": short weekday and clock time, with the date ("Tue, Nov 17, 14:00") when it is over 6 days from `now`. */
export function fmtDayTime(at: At, now = Date.now(), locale?: string): string {
  const t = ms(at);
  const far = Math.abs(t - now) > 6 * 86_400_000;
  const day = new Date(t).toLocaleDateString(locale, { weekday: "short", ...(far ? { month: "short", day: "numeric" } : {}) });
  return `${day} ${fmtClock(new Date(t), { locale })}`;
}

/** A chart's day: "Tue, Nov 17", `offsetFromToday` days ago. */
export function dayLabel(offsetFromToday: number, locale?: string): string {
  const d = new Date();
  d.setDate(d.getDate() - offsetFromToday);
  return d.toLocaleDateString(locale, { weekday: "short", month: "short", day: "numeric" });
}

/** "Nov": the month of a date, for a chart's axis. */
export function fmtMonth(at: At, locale?: string): string {
  return new Date(ms(at)).toLocaleDateString(locale, { month: "short" });
}
