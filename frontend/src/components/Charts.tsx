import { useMemo, useState } from "react";

const SEQ = ["var(--color-seq-0)", "var(--color-seq-1)", "var(--color-seq-2)", "var(--color-seq-3)", "var(--color-seq-4)"];

function level(v: number, max: number) {
  if (v <= 0 || max <= 0) return 0;
  const r = v / max;
  return r > 0.66 ? 4 : r > 0.33 ? 3 : r > 0.12 ? 2 : 1;
}

function dayLabel(offsetFromToday: number) {
  const d = new Date();
  d.setDate(d.getDate() - offsetFromToday);
  return d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

/** GitHub-style contribution grid. `daily` is oldest-first, ending today. */
export function Heatmap({ daily, cell = 11, gap = 3 }: { daily: number[]; cell?: number; gap?: number }) {
  const [hover, setHover] = useState<{ i: number; x: number; y: number } | null>(null);
  const { cols, max, lead } = useMemo(() => {
    // Align so the last column ends on today's weekday.
    const today = new Date().getDay();
    const pad = 6 - today;
    const padded = [...daily, ...Array(pad).fill(-1)];
    const lead = (7 - (padded.length % 7)) % 7;
    const all = [...Array(lead).fill(-1), ...padded];
    const cols: number[][] = [];
    for (let i = 0; i < all.length; i += 7) cols.push(all.slice(i, i + 7));
    return { cols, max: Math.max(1, ...daily), lead };
  }, [daily]);
  const w = cols.length * (cell + gap) - gap;
  const h = 7 * (cell + gap) - gap;
  const monthLabels = useMemo(() => {
    const out: { x: number; label: string }[] = [];
    let last = -1;
    cols.forEach((_, ci) => {
      const idx = ci * 7 - lead;
      const off = daily.length - 1 - Math.max(0, idx);
      const d = new Date();
      d.setDate(d.getDate() - off);
      if (d.getMonth() !== last && d.getDate() <= 7) {
        out.push({ x: ci * (cell + gap), label: d.toLocaleDateString(undefined, { month: "short" }) });
        last = d.getMonth();
      }
    });
    return out;
  }, [cols, lead, daily.length, cell, gap]);

  return (
    <div className="relative">
      <svg width={w} height={h + 16} className="block overflow-visible" role="img" aria-label="Daily commit activity">
        {monthLabels.map((m) => (
          <text key={m.x} x={m.x} y={9} className="fill-fg-3 font-mono" fontSize={9.5}>
            {m.label}
          </text>
        ))}
        <g transform="translate(0,16)">
          {cols.map((col, ci) =>
            col.map((v, ri) => {
              if (v < 0) return null;
              const i = ci * 7 + ri - lead;
              return (
                <rect
                  key={`${ci}-${ri}`}
                  x={ci * (cell + gap)}
                  y={ri * (cell + gap)}
                  width={cell}
                  height={cell}
                  rx={2.5}
                  fill={SEQ[level(v, max)]}
                  stroke={hover?.i === i ? "var(--color-fg)" : "none"}
                  strokeWidth={1}
                  onMouseEnter={() => setHover({ i, x: ci * (cell + gap), y: ri * (cell + gap) })}
                  onMouseLeave={() => setHover(null)}
                />
              );
            }),
          )}
        </g>
      </svg>
      {hover && hover.i >= 0 && hover.i < daily.length && (
        <Tip x={hover.x + cell / 2} y={hover.y + 16}>
          <span className="text-fg">{daily[hover.i]} commit{daily[hover.i] === 1 ? "" : "s"}</span>
          <span className="text-fg-3"> · {dayLabel(daily.length - 1 - hover.i)}</span>
        </Tip>
      )}
      <div className="mt-2 flex items-center gap-1.5 font-mono text-[10px] text-fg-3">
        less
        {SEQ.map((c) => (
          <span key={c} className="inline-block size-2.5 rounded-[2px]" style={{ background: c }} />
        ))}
        more
      </div>
    </div>
  );
}

export function Tip({
  x, y, children, align = "center",
}: {
  x: number | string;
  y: number;
  children: React.ReactNode;
  /** "end" anchors the tooltip's right edge at x, for marks near the right edge. */
  align?: "center" | "end";
}) {
  return (
    <div
      className={`pointer-events-none absolute z-20 -translate-y-full whitespace-nowrap ${align === "end" ? "-translate-x-full" : "-translate-x-1/2"} rounded-lg border border-line-strong bg-ink-800/95 px-2 py-1 font-mono text-[11px] shadow-xl backdrop-blur`}
      style={{ left: x, top: y - 6 }}
    >
      {children}
    </div>
  );
}

/** Weekly commit bars from a daily series (oldest-first). */
export function WeeklyBars({
  daily,
  height = 44,
  weeks = 26,
  className = "",
}: {
  daily: number[];
  height?: number;
  weeks?: number;
  className?: string;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const series = useMemo(() => {
    const out: number[] = [];
    for (let w = weeks - 1; w >= 0; w--) {
      const end = daily.length - w * 7;
      const start = Math.max(0, end - 7);
      out.push(daily.slice(start, end).reduce((a, b) => a + b, 0));
    }
    return out;
  }, [daily, weeks]);
  const max = Math.max(1, ...series);
  const bw = 100 / weeks;
  return (
    <div className={`relative ${className}`} onMouseLeave={() => setHover(null)}>
      <svg viewBox={`0 0 100 ${height}`} preserveAspectRatio="none" className="block w-full" style={{ height }}>
        <line x1={0} x2={100} y1={height - 0.5} y2={height - 0.5} stroke="var(--color-line-strong)" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        {series.map((v, i) => {
          const h = v === 0 ? 0 : Math.max(2, (v / max) * (height - 4));
          return (
            <g key={i} onMouseEnter={() => setHover(i)}>
              <rect x={i * bw} y={0} width={bw} height={height} fill="transparent" />
              {h > 0 && (
                <rect
                  x={i * bw + bw * 0.18}
                  y={height - h}
                  width={bw * 0.64}
                  height={h}
                  rx={0.6}
                  fill={hover === i ? "var(--color-fg)" : "var(--color-accent)"}
                  opacity={hover === null || hover === i ? 0.9 : 0.45}
                />
              )}
            </g>
          );
        })}
      </svg>
      {hover !== null && (
        <Tip x={`${((hover + 0.5) / weeks) * 100}%`} y={0}>
          <span className="text-fg">{series[hover]} commits</span>
          <span className="text-fg-3"> · week of {dayLabel((weeks - 1 - hover) * 7 + 6)}</span>
        </Tip>
      )}
    </div>
  );
}

/** Horizontal share bars — single hue; label text stays in ink tokens. */
export function ShareBars({ items, unit = "files" }: { items: [string, number][]; unit?: string }) {
  const total = items.reduce((a, [, n]) => a + n, 0) || 1;
  return (
    <div className="space-y-1.5">
      {items.map(([label, n]) => (
        <div key={label} className="group grid grid-cols-[88px_1fr_40px] items-center gap-2 text-[12px]" title={`${n} ${unit}`}>
          <span className="truncate text-fg-2">{label}</span>
          <div className="h-1.5 overflow-hidden rounded-full bg-ink-700">
            <div className="h-full rounded-full bg-accent/80 transition-all group-hover:bg-accent" style={{ width: `${(n / total) * 100}%` }} />
          </div>
          <span className="text-right font-mono text-[11px] text-fg-3">{Math.round((n / total) * 100)}%</span>
        </div>
      ))}
    </div>
  );
}
