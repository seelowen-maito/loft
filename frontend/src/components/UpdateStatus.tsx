import { useCallback, useEffect, useState } from "react";
import { api, type UpdateStatus as Status } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx } from "../lib/format";

/** After a restart the old status is stale until the new process answers: ask again, a few times. */
async function refetchUntilNew(set: (s: Status) => void) {
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    try {
      const s = await api.update();
      set(s);
      if (s.phase !== "restart") return;
    } catch {
      /* still restarting */
    }
  }
}

/** Navbar light for auto-update (LOFT_AUTO_UPDATE); hidden when it's off. */
export function UpdateStatus({ live }: { live: boolean }) {
  const [s, setS] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  // Refetch on (re)connect: a restart after an update drops the event stream.
  useEffect(() => {
    if (live) api.update().then(setS).catch(() => {});
  }, [live]);
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "update") setS(e.status);
    }, []),
  );
  if (!s) return null;

  const agents = `${s.running_agents} agent${s.running_agents === 1 ? "" : "s"}`;
  const views: Partial<Record<string, [string, string, string]>> = {
    up_to_date: ["bg-good", "up to date", `Running ${s.current}, the latest commit.`],
    waiting: ["bg-warn animate-pulse-soft", "update available, waiting for agents", `${s.latest} is available. Installs once ${agents} finish.`],
    updating: ["bg-warn animate-pulse-soft", "updating…", `Building ${s.latest}; loft restarts when done.`],
    restart: ["bg-warn animate-pulse-soft", "restart to update", s.why ?? "A newer build is on disk than the one running."],
    error: ["bg-bad", "update failed", s.error ?? "Update failed."],
  };
  // Hidden when disabled, and for anything unexpected (e.g. an older backend without /api/update).
  const view = views[s.phase];
  if (!view) return null;
  const [dot, label, title] = view;

  const restart = () => {
    setBusy(true);
    api
      .restart()
      .then(() => refetchUntilNew(setS))
      .catch((e) => alert((e as Error).message))
      .finally(() => setBusy(false));
  };
  return (
    <span className="flex items-center gap-1.5 font-mono text-[11px] text-fg-3" title={title}>
      <span className={cx("size-1.5 rounded-full", dot)} />
      {label}
      {s.phase === "restart" && (
        <button type="button" className="text-accent hover:underline disabled:opacity-50" disabled={busy || s.running_agents > 0} onClick={restart}>
          Restart
        </button>
      )}
    </span>
  );
}
