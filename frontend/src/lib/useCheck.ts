import { useCallback, useEffect, useRef, useState } from "react";
import { errorText } from "./useAction.ts";

type Check<T> = { data: T | null; err: string | null };

/** What a Settings card shows after a check: a good one replaces the data and clears an earlier error; a failed one keeps the last data and shows its error. */
export function afterCheck<T>(prev: Check<T>, r: { ok: T } | { error: unknown }): Check<T> {
  return "ok" in r ? { data: r.ok, err: null } : { data: prev.data, err: errorText(r.error) };
}

/** Whether a card shows its skeleton: only while its first, automatic check is out. A manual card has none. */
export function isLoading(s: Check<unknown>, manual: boolean): boolean {
  return !manual && s.data === null && s.err === null;
}

/** A check a Settings card runs when it opens and again on its action ("Check again"). `loading` is only the first one.
 *  With `manual`, it runs only on the action: for a check that can raise prompts on the Mac. */
export function useCheck<T>(fn: () => Promise<T>, { manual = false } = {}) {
  const [state, setState] = useState<Check<T>>({ data: null, err: null });
  const [busy, setBusy] = useState(false);
  const latest = useRef(fn);
  latest.current = fn;
  // Only the newest run shows: a slow earlier check ("Check again" right after the automatic one) can't overwrite it.
  const runs = useRef(0);
  const run = useCallback(async () => {
    const n = ++runs.current;
    setBusy(true);
    try {
      const ok = await latest.current();
      if (n === runs.current) setState((s) => afterCheck(s, { ok }));
    } catch (error) {
      if (n === runs.current) setState((s) => afterCheck(s, { error }));
    } finally {
      if (n === runs.current) setBusy(false);
    }
  }, []);
  useEffect(() => {
    if (!manual) run();
  }, [run, manual]);
  return { data: state.data, err: state.err, busy, run, loading: isLoading(state, manual) };
}
