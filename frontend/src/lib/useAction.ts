import { useCallback, useRef, useState } from "react";

/** What a failed request says, for the user: an Error's message, or the thing itself. */
export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * One async action behind a button: `busy` while it runs (a second `run` meanwhile is ignored), `error` if it threw,
 * cleared on the next run. `run` resolves to what `fn` returned, or undefined when it failed or was ignored.
 */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const running = useRef(false);
  const run = useCallback(async <T>(fn: () => Promise<T>): Promise<T | undefined> => {
    if (running.current) return undefined;
    running.current = true;
    setBusy(true);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(errorText(e));
      return undefined;
    } finally {
      running.current = false;
      setBusy(false);
    }
  }, []);
  return { busy, error, setError, run };
}
