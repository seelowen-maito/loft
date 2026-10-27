import { useCallback, useEffect, useState } from "react";
import { MemoryStick, Thermometer } from "lucide-react";
import { api, type SystemReading } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx } from "../lib/format";

const tone = (l: SystemReading["level"]) => (l === "hot" ? "text-bad" : l === "warm" ? "text-warn" : "text-fg-3");
const memTone = (l: SystemReading["mem_level"]) => (l === "critical" ? "text-bad" : l === "high" ? "text-warn" : "text-fg-3");
const gb = (bytes: number) => Math.round(bytes / 2 ** 30);

/** Navbar readout of this Mac's CPU temperature and memory. Each is hidden where it can't be read. */
export function TempMeter({ live }: { live: boolean }) {
  const [r, setR] = useState<SystemReading | null>(null);
  useEffect(() => {
    if (live) api.system().then(setR).catch(() => {});
  }, [live]);
  useServerEvents(useCallback((e: ServerEvent) => e.type === "system" && setR(e.system), []));
  if (!r) return null;
  const used = r.mem_used_bytes;
  const total = r.mem_total_bytes;
  return (
    <>
      {used != null && total != null && (
        <span
          className={cx("flex items-center gap-1 font-mono text-[11px] tabular-nums", memTone(r.mem_level))}
          title={`Memory used ${(used / 2 ** 30).toFixed(1)} of ${gb(total)} GB (${Math.round((used / total) * 100)} %), sampled every 30 s`}
        >
          <MemoryStick className="size-3.5" />
          RAM {gb(used)}/{gb(total)} GB
        </span>
      )}
      {r.cpu_temp_c != null && (
        <span
          className={cx("flex items-center gap-1 font-mono text-[11px] tabular-nums", tone(r.level))}
          title={`CPU ${r.cpu_temp_c.toFixed(1)} °C (hottest die sensor), sampled every 30 s`}
        >
          <Thermometer className="size-3.5" />
          {Math.round(r.cpu_temp_c)}°
        </span>
      )}
    </>
  );
}
