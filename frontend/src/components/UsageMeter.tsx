import { useEffect, useRef, useState } from "react";
import { Gauge } from "lucide-react";
import { api, type Usage } from "../lib/api";
import { cx } from "../lib/format";
import { resetText } from "../lib/usage";

const POLL_MS = 120_000;

const tone = (p: number) => (p >= 90 ? "text-bad" : p >= 75 ? "text-warn" : "text-fg-3");

/** Navbar readout of Claude plan usage, as in Claude Code's /usage: the 5-hour session and the week. */
export function UsageMeter({ live }: { live: boolean }) {
  const [u, setU] = useState<Usage | null>(null);
  // The counter whose reset tooltip is open: hovered with a mouse, or tapped on touch.
  const [open, setOpen] = useState<string | null>(null);
  const pointer = useRef("mouse");
  const root = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!live) return;
    const load = () => api.usage().then(setU).catch(() => {});
    load();
    const t = setInterval(load, POLL_MS);
    return () => clearInterval(t);
  }, [live]);
  // A tap anywhere else closes a tapped-open tooltip.
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(null);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, [open]);
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
    <span ref={root} className="flex items-center gap-1.5 font-mono text-[11px] tabular-nums text-fg-3">
      <Gauge className="size-3.5" />
      {shown.map((w, i) => (
        <span key={w.key} className="flex items-center gap-1">
          {i > 0 && <span className="text-fg-3/50">·</span>}
          <button
            type="button"
            className="relative flex items-center gap-1 rounded hover:text-fg-2"
            aria-expanded={open === w.key}
            onPointerDown={(e) => (pointer.current = e.pointerType)}
            onPointerEnter={(e) => e.pointerType === "mouse" && setOpen(w.key)}
            onPointerLeave={(e) => e.pointerType === "mouse" && setOpen(null)}
            onClick={() => setOpen((o) => (o === w.key && pointer.current !== "mouse" ? null : w.key))}
          >
            {w.key === "five_hour" ? "session" : "week"}
            <span className={cx(tone(w.percent))}>{Math.round(w.percent)}%</span>
            {open === w.key && (
              <span
                role="tooltip"
                className="pointer-events-none absolute right-0 top-full z-30 mt-2 flex flex-col items-start whitespace-nowrap rounded-lg border border-line-strong bg-ink-800/95 px-2 py-1 text-left font-mono text-[11px] shadow-xl backdrop-blur"
              >
                <span className="text-fg-2">
                  {w.label.charAt(0).toUpperCase() + w.label.slice(1)}: {Math.round(w.percent)}% used
                </span>
                <span className="text-fg-3">{resetText(w.resets_at, now)}</span>
              </span>
            )}
          </button>
        </span>
      ))}
    </span>
  );
}
