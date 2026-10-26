import { useCallback, useEffect, useState } from "react";
import { Thermometer } from "lucide-react";
import { api, type SystemReading } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx } from "../lib/format";

const tone = (l: SystemReading["level"]) => (l === "hot" ? "text-bad" : l === "warm" ? "text-warn" : "text-fg-3");

/** Navbar readout of this Mac's CPU temperature. Hidden where it can't be read. */
export function TempMeter({ live }: { live: boolean }) {
  const [r, setR] = useState<SystemReading | null>(null);
  useEffect(() => {
    if (live) api.system().then(setR).catch(() => {});
  }, [live]);
  useServerEvents(useCallback((e: ServerEvent) => e.type === "system" && setR(e.system), []));
  if (r?.cpu_temp_c == null) return null;
  return (
    <span
      className={cx("flex items-center gap-1 font-mono text-[11px] tabular-nums", tone(r.level))}
      title={`CPU ${r.cpu_temp_c.toFixed(1)} °C (hottest die sensor), sampled every 30 s`}
    >
      <Thermometer className="size-3.5" />
      {Math.round(r.cpu_temp_c)}°
    </span>
  );
}
