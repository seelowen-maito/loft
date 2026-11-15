import { useEffect, useState } from "react";

/** The current time in ms, refreshed every `every` ms while `active`. Only the component using it re-renders. */
export function useNow(every: number, active = true): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), every);
    return () => clearInterval(t);
  }, [every, active]);
  return now;
}
