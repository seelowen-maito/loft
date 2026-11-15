import { useCallback, useEffect, useState } from "react";
import { api, type UpdateStatus as Status } from "../lib/api";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx } from "../lib/format";
import { restartDisabled } from "../lib/restartButton";
import { usePoll } from "../hooks/usePoll";
import { useAction } from "../lib/useAction";
import { ActionButton } from "./ActionButton";
import { ErrorLine } from "./Callout";

/** Navbar light for auto-update (LOFT_AUTO_UPDATE); hidden when it's off. */
export function UpdateStatus({ live }: { live: boolean }) {
  const [s, setS] = useState<Status | null>(null);
  const ask = useAction();
  // Asked, until the phase changes: the request returns before the restart happens.
  const [sent, setSent] = useState(false);
  // Refetch on (re)connect: a restart after an update drops the event stream.
  useEffect(() => {
    if (live) api.update().then(setS).catch(() => {});
  }, [live]);
  // While a restart is due (or after a restart on its own) the page can't be told: ask again until it changes.
  const restarting = s?.phase === "restart";
  useEffect(() => {
    if (!restarting) setSent(false);
  }, [restarting]);
  usePoll(restarting ? () => api.update().then(setS).catch(() => {}) : null, 3000);
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
    blocked: ["bg-warn", "can't update", s.why ?? "Loft's checkout can't be updated."],
  };
  // Hidden when disabled, and for anything unexpected (e.g. an older backend without /api/update).
  const view = views[s.phase];
  if (!view) return null;
  const [dot, label, title] = view;

  const restart = () => ask.run(async () => {
    await api.restart();
    setSent(true);
  });
  const working = ask.busy || (sent && !s.why?.startsWith("restart failed"));
  return (
    <span className="flex items-center gap-1.5 font-mono text-[11px] text-fg-3" title={title}>
      <span className={cx("size-1.5 rounded-full", dot)} />
      {label}
      {s.phase === "restart" && (
        <ActionButton type="button" variant="link" pending={working} pendingLabel="Restarting…" disabled={restartDisabled(s, working)} onClick={restart}>
          Restart
        </ActionButton>
      )}
      <ErrorLine error={ask.error && `Couldn't restart: ${ask.error}`} className="text-[11px]" />
    </span>
  );
}
