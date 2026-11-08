/** Keeps the newest of overlapping loads of one value: a load's answer is shown only when no load or set started
 *  after it, and nothing is shown once the owner is gone. A burst of reloads (one Resume saves its Delivery several
 *  times, and each save emits an event) can't land an older answer last. */
export function latestOnly<T>(apply: (v: T) => void) {
  let n = 0;
  let live = true;
  return {
    /** Starts showing answers again (a remount). */
    start() {
      live = true;
    },
    /** The owner is gone: nothing in flight is shown, nor anything set later. */
    stop() {
      live = false;
      n++;
    },
    /** Shows `p`'s answer if it is still the newest when it arrives. A failed load shows nothing. */
    load(p: Promise<T>) {
      const mine = ++n;
      p.then(
        (v) => {
          if (live && mine === n) apply(v);
        },
        () => {},
      );
    },
    /** Shows a value known now (an action's answer): loads started before it are older, so they're dropped. */
    set(v: T) {
      n++;
      if (live) apply(v);
    },
  };
}
