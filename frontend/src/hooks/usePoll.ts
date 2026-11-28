import { useEffect, useRef } from "react";

/** Calls `fn` now and every `ms` while it's set; null stops. A new `fn` each render doesn't restart the clock. */
export function usePoll(fn: (() => unknown) | null, ms: number) {
  const latest = useRef(fn);
  latest.current = fn;
  const on = !!fn;
  useEffect(() => {
    if (!on) return;
    const tick = () => latest.current?.();
    tick();
    const t = setInterval(tick, ms);
    return () => clearInterval(t);
  }, [on, ms]);
}
