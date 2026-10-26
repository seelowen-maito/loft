import { useCallback, useEffect, useState } from "react";
import { api, type UpdateStatus as Status } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx } from "../lib/format";

/** Navbar light for auto-update (LOFT_AUTO_UPDATE); hidden when it's off. */
export function UpdateStatus({ live }: { live: boolean }) {
  const [s, setS] = useState<Status | null>(null);
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
    error: ["bg-bad", "update failed", s.error ?? "Update failed."],
  };
  // Hidden when disabled, and for anything unexpected (e.g. an older backend without /api/update).
  const view = views[s.phase];
  if (!view) return null;
  const [dot, label, title] = view;

  return (
    <span className="flex items-center gap-1.5 font-mono text-[11px] text-fg-3" title={title}>
      <span className={cx("size-1.5 rounded-full", dot)} />
      {label}
    </span>
  );
}
