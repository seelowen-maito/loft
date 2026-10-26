import { useState } from "react";
import { MessagesSquare, Pause, Play, Square } from "lucide-react";
import { api, type GrillSession } from "../../lib/api";
import { cx } from "../../lib/format";

/** The running interview above the composer: where it is, and Pause / Resume / End (the app's buttons, same endpoint). */
export function GrillBar({ session }: { session: GrillSession }) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const paused = session.state === "paused";

  const send = async (command: "pause" | "resume" | "end") => {
    setBusy(true);
    setError(null);
    try {
      await api.grillCommand(session.session_id, command);
      setConfirming(false);
    } catch (e) {
      setError((e as Error).message === "grill_closed" ? "This interview has already ended." : (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const where = session.index != null && session.total ? `Question ${session.index} of ${session.total}` : `${session.answered} answered`;

  return (
    <>
      <div className={cx("mb-3 flex items-center gap-3 rounded-xl border px-3 py-2", paused ? "border-warn/30 bg-warn/[0.07]" : "border-accent/25 bg-accent/[0.05]")} role="status">
        <MessagesSquare className={cx("size-4 shrink-0", paused ? "text-warn" : "text-accent")} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] text-fg">
            {paused ? "Interview paused" : "Interview"} · <span className="text-fg-2">{session.topic}</span>
          </div>
          <div className="truncate font-mono text-[11px] text-fg-3">
            {where} · type an answer, or “skip”, “pause”, “stop the interview”
          </div>
        </div>
        <button className="btn shrink-0" onClick={() => send(paused ? "resume" : "pause")} disabled={busy} aria-label={paused ? "Resume" : "Pause"} title={paused ? "Resume" : "Pause"}>
          {paused ? <Play className="size-3 fill-current" /> : <Pause className="size-3 fill-current" />} <span className="max-md:hidden">{paused ? "Resume" : "Pause"}</span>
        </button>
        <button className="btn shrink-0" onClick={() => setConfirming(true)} disabled={busy} aria-label="End interview" title="End interview">
          <Square className="size-3 fill-current" /> <span className="max-md:hidden">End interview</span>
        </button>
      </div>
      {error && <div className="-mt-2 mb-2 text-[12px] text-bad">{error}</div>}

      {confirming && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={() => setConfirming(false)}>
          <div className="panel w-full max-w-sm space-y-4 bg-ink-900 p-5" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-[15px] font-medium">End the interview?</h3>
            <p className="text-[13px] text-fg-2">
              The questions stop now.{" "}
              {session.answered === 0 ? "Nothing has been answered yet" : session.answered === 1 ? "Your answer so far is kept" : `Your ${session.answered} answers so far are kept`}, and the agent writes a summary of what was covered.
            </p>
            <div className="flex justify-end gap-2">
              <button className="btn-ghost" onClick={() => setConfirming(false)}>
                Keep going
              </button>
              <button className="btn-primary" disabled={busy} onClick={() => send("end")}>
                End interview
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
