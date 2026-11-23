import { useEffect, useSyncExternalStore } from "react";
import { api, type Info } from "../lib/api";
import { onConnection } from "../lib/events";
import { offered, type Offered } from "../lib/platform";
import { createSharedValue } from "../lib/sharedValue";

// `/api/info`, read once per page load and shared by every component. A failed read (the backend restarting at page
// load, a flaky network) is retried each time the live event stream opens.
const info = createSharedValue(() => api.info());
let attached = false;

/** `/api/info` once it has answered, for every component that reads it. */
export function useInfo(): Info | null {
  useEffect(() => {
    if (!attached) {
      attached = true;
      onConnection((live) => live && info.ensure());
    }
    info.ensure();
  }, []);
  return useSyncExternalStore(info.subscribe, info.get);
}

/** Reads `/api/info` again, for a value that changes while Loft runs (whether a GitHub token is set). */
export const reloadInfo = () => info.reload();

/** The Mac-only features the server offers: none until it says. */
export function useOffered(): Offered {
  return offered(useInfo());
}
