import { useCallback, useEffect, useRef, useState } from "react";

type Check<T> = { data: T | null; err: string | null };

/** What a Settings card shows after a check: a good one replaces the data and clears an earlier error; a failed one keeps the last data and shows its error. */
export function afterCheck<T>(prev: Check<T>, r: { ok: T } | { error: unknown }): Check<T> {
  return "ok" in r ? { data: r.ok, err: null } : { data: prev.data, err: r.error instanceof Error ? r.error.message : String(r.error) };
}

/** A check a Settings card runs when it opens and again on its action ("Check again"). `loading` is only the first one. */
export function useCheck<T>(fn: () => Promise<T>) {
  const [state, setState] = useState<Check<T>>({ data: null, err: null });
  const [busy, setBusy] = useState(false);
  const latest = useRef(fn);
  latest.current = fn;
  const run = useCallback(async () => {
    setBusy(true);
    try {
      const ok = await latest.current();
      setState((s) => afterCheck(s, { ok }));
    } catch (error) {
      setState((s) => afterCheck(s, { error }));
    } finally {
      setBusy(false);
    }
  }, []);
  useEffect(() => {
    run();
  }, [run]);
  return { data: state.data, err: state.err, busy, run, loading: state.data === null && state.err === null };
}
