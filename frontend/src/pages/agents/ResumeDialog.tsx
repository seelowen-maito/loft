import { useEffect, useState } from "react";
import { History, Loader2, X } from "lucide-react";
import { api, type PastSession } from "../../lib/api";
import { cx, fmtNum } from "../../lib/format";
import { contextWarns } from "../../lib/context";

/** `/resume`: the agent's past Sessions, newest first; picking one makes it the agent's Session. */
export function ResumeDialog({ agentId, running, onClose }: { agentId: string; running: boolean; onClose: () => void }) {
  const [list, setList] = useState<{ sessions: PastSession[]; current: string | null } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    api.sessions(agentId).then(setList, (e) => setErr((e as Error).message));
  }, [agentId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const resume = async (id: string) => {
    setBusy(id);
    setErr(null);
    try {
      await api.resumeSession(agentId, id);
      onClose();
    } catch (e) {
      setErr((e as Error).message);
      setBusy(null);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div role="dialog" aria-label="Resume a session" className="relative flex max-h-[80vh] w-full max-w-[560px] animate-rise flex-col overflow-clip rounded-3xl border border-line-strong bg-ink-900 shadow-2xl">
        <div className="flex items-start justify-between px-6 pt-6">
          <div>
            <div className="eyebrow">/resume</div>
            <h2 className="mt-1.5 font-serif text-[28px] leading-none">Resume a session</h2>
            <p className="mt-2 text-[12.5px] text-fg-3">
              The agent's next run picks up where that Session left off. Its whole context goes back into the prompt cache at full price, so a big one costs more.
            </p>
          </div>
          <button className="btn-ghost" onClick={onClose} aria-label="Close"><X className="size-4" /></button>
        </div>

        <div className="mt-4 min-h-0 flex-1 overflow-y-auto px-3 pb-4">
          {running && <p className="px-3 pb-2 text-[12.5px] text-warn">Stop the agent first.</p>}
          {err && <p className="px-3 pb-2 text-[12.5px] text-bad">{err}</p>}
          {!list && !err && (
            <p className="flex items-center gap-2 px-3 py-4 text-[12.5px] text-fg-3"><Loader2 className="size-3.5 animate-spin" /> Reading the transcripts…</p>
          )}
          {list && list.sessions.length === 0 && <p className="px-3 py-4 text-[12.5px] text-fg-3">No past Sessions for this agent yet.</p>}
          {list?.sessions.map((s) => {
            const current = s.id === list.current;
            return (
              <button
                key={s.id}
                type="button"
                className="flex w-full items-start gap-3 rounded-xl px-3 py-2.5 text-left hover:bg-white/5 disabled:opacity-50 disabled:hover:bg-transparent"
                disabled={running || current || busy !== null}
                onClick={() => resume(s.id)}
              >
                {busy === s.id ? <Loader2 className="mt-0.5 size-3.5 shrink-0 animate-spin" /> : <History className="mt-0.5 size-3.5 shrink-0 text-fg-3" />}
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline gap-2 text-[11.5px] text-fg-3">
                    <span>{new Date(s.started_at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}</span>
                    {current && <span className="text-accent">current</span>}
                    {s.context_tokens != null && (
                      <span className={cx("ml-auto font-mono tabular-nums", contextWarns(s.context_tokens) && "text-warn")}>context {fmtNum(s.context_tokens)}</span>
                    )}
                  </span>
                  <span className="mt-0.5 line-clamp-2 text-[13px] text-fg">
                    {s.from_agent && <span className="text-fg-3">{s.from_agent}: </span>}
                    {s.first_message}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
