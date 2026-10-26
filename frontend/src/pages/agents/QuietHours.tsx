import { Plus, X } from "lucide-react";
import type { QuietWindow } from "../../lib/api";
import { DAYS, cx, fmtInterval, isQuiet } from "../../lib/format";
import { Toggle } from "../Settings";

const WORK_HOURS: QuietWindow = { days: [0, 1, 2, 3, 4], start: "09:00", end: "17:00" };
const SLOTS = 48; // half hours

/** Optional weekly windows when autonomous wake-ups are held, plus the schedule they produce. */
export function QuietHours({ value, onChange, interval }: { value: QuietWindow[]; onChange: (v: QuietWindow[]) => void; interval: number }) {
  const update = (i: number, patch: Partial<QuietWindow>) => onChange(value.map((w, j) => (j === i ? { ...w, ...patch } : w)));
  const toggleDay = (i: number, d: number) => {
    const days = value[i].days;
    update(i, { days: days.includes(d) ? days.filter((x) => x !== d) : [...days, d].sort((a, b) => a - b) });
  };
  return (
    <div className="space-y-3">
      <Toggle
        on={value.length > 0}
        onChange={(on) => onChange(on ? [WORK_HOURS] : [])}
        label="Pause during working hours"
        hint="Hold autonomous wake-ups while you need your Claude usage yourself. It catches up when the window ends; messages still go through."
      />
      {value.length > 0 && (
        <>
          {value.map((w, i) => (
            <div key={i} className="flex flex-wrap items-center gap-2 rounded-xl border border-line p-2.5">
              <div className="flex gap-1">
                {DAYS.map((d, n) => (
                  <button
                    key={d}
                    type="button"
                    onClick={() => toggleDay(i, n)}
                    className={cx(
                      "w-8 rounded-md py-1 text-[11px] transition",
                      w.days.includes(n) ? "bg-accent/15 text-accent" : "bg-white/[0.04] text-fg-3 hover:text-fg",
                    )}
                  >
                    {d.slice(0, 2)}
                  </button>
                ))}
              </div>
              <div className="flex items-center gap-1.5">
                <input type="time" className="input w-[6.5rem] py-1" value={w.start} onChange={(e) => e.target.value && update(i, { start: e.target.value })} />
                <span className="text-fg-3">–</span>
                <input type="time" className="input w-[6.5rem] py-1" value={w.end} onChange={(e) => e.target.value && update(i, { end: e.target.value })} />
              </div>
              {value.length > 1 && (
                <button type="button" className="btn-ghost ml-auto p-1" title="Remove window" onClick={() => onChange(value.filter((_, j) => j !== i))}>
                  <X className="size-3.5" />
                </button>
              )}
            </div>
          ))}
          <div className="flex items-center justify-between gap-3">
            <button type="button" className="btn-ghost" onClick={() => onChange([...value, { days: [5, 6], start: "00:00", end: "00:00" }])}>
              <Plus className="size-3.5" /> Add window
            </button>
            <span className="text-[11px] text-fg-3">Same start and end = all day. Times are this Mac's.</span>
          </div>
          <ScheduleGrid windows={value} interval={interval} />
        </>
      )}
    </div>
  );
}

/** The week at a glance: when the agent wakes up on its own, and when it's paused. */
export function ScheduleGrid({ windows, interval }: { windows: QuietWindow[]; interval: number }) {
  const week = DAYS.map((_, day) => Array.from({ length: SLOTS }, (_, s) => !isQuiet(windows, day, s * 30 + 15)));
  const hours = week.flat().filter(Boolean).length / 2;
  return (
    <div className="rounded-xl border border-line p-3">
      <div className="mb-2.5 flex items-baseline justify-between">
        <span className="eyebrow">Autonomy schedule</span>
        <span className="font-mono text-[10.5px] text-fg-3">custom · {fmtInterval(interval)} · {hours}h a week</span>
      </div>
      <div className="space-y-1">
        {week.map((slots, day) => (
          <div key={day} className="flex items-center gap-2">
            <span className="w-7 font-mono text-[10.5px] text-fg-3">{DAYS[day]}</span>
            <div className="flex h-3 flex-1 gap-px">
              {slots.map((active, s) => (
                <span
                  key={s}
                  className={cx("flex-1", active ? "bg-accent/70" : "bg-white/[0.08]", s === 0 && "rounded-l-sm", s === SLOTS - 1 && "rounded-r-sm")}
                />
              ))}
            </div>
          </div>
        ))}
        <div className="flex justify-between pl-9 font-mono text-[10px] text-fg-3">
          {["00", "06", "12", "18", "24"].map((h) => <span key={h}>{h}</span>)}
        </div>
      </div>
      <div className="mt-2.5 flex gap-4 text-[11px] text-fg-3">
        <span className="flex items-center gap-1.5"><span className="inline-block size-2.5 rounded-[2px] bg-accent/70" /> wakes up {fmtInterval(interval)}</span>
        <span className="flex items-center gap-1.5"><span className="inline-block size-2.5 rounded-[2px] bg-white/[0.08]" /> paused</span>
      </div>
    </div>
  );
}
