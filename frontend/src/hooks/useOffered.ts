import { useEffect, useState } from "react";
import { api, type Info } from "../lib/api";
import { offered, type Offered } from "../lib/platform";

// `/api/info` is read once per page load and shared: what the server offers doesn't change while it runs.
let pending: Promise<Info> | null = null;
let known: Info | null = null;

/** The Mac-only features the server offers: none until it says. */
export function useOffered(): Offered {
  const [info, setInfo] = useState(known);
  useEffect(() => {
    if (known) return;
    (pending ??= api.info()).then(
      (i) => {
        known = i;
        setInfo(i);
      },
      () => {
        pending = null;
      },
    );
  }, []);
  return offered(info);
}
