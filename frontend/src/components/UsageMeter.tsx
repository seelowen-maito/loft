import { useEffect, useState } from "react";
import { Gauge } from "lucide-react";
import { api, type Usage } from "../lib/api";
import { cx, timeAgo } from "../lib/format";

const POLL_MS = 120_000;

const tone = (p: number) => (p >= 90 ? "text-bad" : p >= 75 ? "text-warn" : "text-fg-3");

/** Navbar readout of Claude plan usage, as in Claude Code's /usage: the 5-hour session and the week. */
export function UsageMeter({ live }: { live: boolean }) {
  const [u, setU] = useState<Usage | null>(null);
  useEffect(() => {
    if (!live) return;
    const load = () => api.usage().then(setU).catch(() => {});
    load();
    const t = setInterval(load, POLL_MS);
    return () => clearInterval(t);
  }, [live]);
  // Hidden for anything unexpected (e.g. an older backend without /api/usage serves index.html).
  if (!u || !Array.isArray(u.windows)) return null;

  if (u.error) {
    return (
      <span className="flex items-center gap-1 font-mono text-[11px] text-fg-3" title={`Claude usage unavailable: ${u.error}`}>
        <Gauge className="size-3.5" /> usage —
      </span>
    );
  }
  const shown = u.windows.filter((w) => w.key === "five_hour" || w.key === "seven_day");
  if (!shown.length) return null;
  const title = u.windows
    .map((w) => `${w.label}: ${Math.round(w.percent)}% used${w.resets_at ? `, resets ${timeAgo(w.resets_at)} (${new Date(w.resets_at).toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false })})` : ""}`)
    .join("\n");

  return (
    <span className="flex items-center gap-1.5 font-mono text-[11px] tabular-nums text-fg-3" title={title}>
      <Gauge className="size-3.5" />
      {shown.map((w, i) => (
        <span key={w.key} className="flex items-center gap-1">
          {i > 0 && <span className="text-fg-3/50">·</span>}
          {w.key === "five_hour" ? "session" : "week"}
          <span className={cx(tone(w.percent))}>{Math.round(w.percent)}%</span>
        </span>
      ))}
    </span>
  );
}
