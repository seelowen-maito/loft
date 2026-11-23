/** One copy of a server value for every component that reads it, such as `/api/info`. No imports, so `node --test`
 * can load it. */
export function createSharedValue<T>(load: () => Promise<T>) {
  let value: T | null = null;
  let loading: Promise<void> | null = null;
  const listeners = new Set<() => void>();

  const store = {
    get: () => value,
    subscribe(l: () => void) {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    /** Loads it unless it's loaded or loading. A failed load leaves it as it was, for the next `ensure` to retry. */
    ensure() {
      if (value === null && !loading) store.reload();
    },
    /** Loads it again, keeping the current value until the new one arrives; every subscriber hears of it. */
    reload(): Promise<void> {
      if (loading) return loading;
      const p = load().then(
        (v) => {
          value = v;
          listeners.forEach((l) => l());
        },
        () => {},
      ).finally(() => {
        loading = null;
      });
      loading = p;
      return p;
    },
  };
  return store;
}
