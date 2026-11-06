import { useEffect, useState } from "react";
import { Gauge } from "lucide-react";
import { api, type Usage } from "../lib/api";
import { cx } from "../lib/format";
import { HoverTip } from "./HoverTip";
import { resetText } from "../lib/usage";

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
  const now = Date.now();

  return (
    <span className="flex items-center gap-1.5 font-mono text-[11px] tabular-nums text-fg-3">
      <Gauge className="size-3.5" />
      {shown.map((w, i) => (
        <span key={w.key} className="flex items-center gap-1">
          {i > 0 && <span className="text-fg-3/50">·</span>}
          <HoverTip
            tip={
              <>
                <span className="text-fg-2">
                  {w.label.charAt(0).toUpperCase() + w.label.slice(1)}: {Math.round(w.percent)}% used
                </span>
                <span className="text-fg-3">{resetText(w.resets_at, now)}</span>
              </>
            }
          >
            {w.key === "five_hour" ? "session" : "week"}
            <span className={cx(tone(w.percent))}>{Math.round(w.percent)}%</span>
          </HoverTip>
        </span>
      ))}
    </span>
  );
}
