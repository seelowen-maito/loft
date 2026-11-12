import type { UpdateStatus } from "./api";

/** After a restart the old status is stale until the new process answers: ask again, once a second, up to `tries`
 * times. Stops when the phase leaves "restart", or when the restart failed (so the button works again). */
export async function refetchUntilNew(
  fetchStatus: () => Promise<UpdateStatus>,
  set: (s: UpdateStatus) => void,
  tries = 30,
  wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms)),
) {
  for (let i = 0; i < tries; i++) {
    await wait(1000);
    try {
      const s = await fetchStatus();
      set(s);
      if (s.phase !== "restart" || s.why?.startsWith("restart failed")) return;
    } catch {
      /* still restarting */
    }
  }
}
