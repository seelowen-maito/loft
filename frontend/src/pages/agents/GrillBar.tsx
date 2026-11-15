import { useState } from "react";
import { useNavigate } from "react-router";
import { ArrowDownToLine, Maximize2, MessagesSquare, Pause, Play, Send, Square } from "lucide-react";
import { api, grillError, type GrillSession } from "../../lib/api";
import { cx } from "../../lib/format";
import { ConfirmDialog } from "../../components/Dialog";
import { interviewPath, nextQuestion, questionAnchor } from "../../lib/interview";
import { ActionButton } from "../../components/ActionButton";
import { ErrorLine } from "../../components/Callout";
import { useAction } from "../../lib/useAction";

/** The round bar above the composer: which round, how much of it is answered, and Send / Pause / Resume / End (the app's buttons, same endpoint).
 * In the chat it also opens the full-screen interview; `fullScreen` is the bar on that page. */
type Command = "send" | "pause" | "resume" | "end";

export function GrillBar({ session, fullScreen = false }: { session: GrillSession; fullScreen?: boolean }) {
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);
  const commanding = useAction();
  // The command running, so its button shows the spinner.
  const [command, setCommand] = useState<Command | null>(null);
  const paused = session.state === "paused";
  const busy = commanding.busy;

  const send = (c: Command) => {
    if (busy) return;
    setCommand(c);
    return commanding.run(async () => {
      await api.grillCommand(session.session_id, c);
      setConfirming(false);
    });
  };

  const answered = session.answered ?? 0;
  const total = session.total ?? 0;
  const next = nextQuestion(session);
  const where =
    session.round == null ? "Preparing the first round…"
    : session.waiting ? `Round ${session.round} sent · the agent is preparing the next one`
    : next ? `Round ${session.round} · Question ${next.n} of ${next.total}`
    : `Round ${session.round} · all ${total} answered`;

  // Scrolls to the first unanswered question's card and flashes it, so a long chat never loses the round.
  const jump = () => {
    if (!next || session.round == null) return;
    const card = document.querySelector<HTMLElement>(`[data-grill-question="${CSS.escape(questionAnchor(session.session_id, session.round, next.id))}"]`);
    if (!card) return;
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    card.scrollIntoView({ behavior: still ? "auto" : "smooth", block: "center" });
    card.querySelector("textarea")?.focus({ preventScroll: true });
    card.animate([{ boxShadow: "0 0 0 2px var(--color-accent)" }, { boxShadow: "0 0 0 2px transparent" }], { duration: 1800, easing: "ease-out" });
  };
  const pct = total ? Math.round((answered / total) * 100) : 0;

  return (
    <>
      <div className={cx("mb-3 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border px-3 py-2", paused ? "border-warn/30 bg-warn/[0.07]" : "border-accent/25 bg-accent/[0.05]")} role="status">
        <MessagesSquare className={cx("size-4 shrink-0", paused ? "text-warn" : "text-accent")} />
        <div className="min-w-0 flex-1 basis-40">
          <div className="truncate text-[13px] text-fg">
            {paused ? "Interview paused" : "Interview"} · <span className="text-fg-2">{session.topic}</span>
          </div>
          <div className="truncate font-mono text-[11px] text-fg-3">{where}</div>
          {session.round != null && !session.waiting && (
            <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-white/[0.06]" role="progressbar" aria-valuenow={answered} aria-valuemin={0} aria-valuemax={total}>
              <div className={cx("h-full rounded-full transition-all", paused ? "bg-warn" : "bg-accent")} style={{ width: `${pct}%` }} />
            </div>
          )}
        </div>
        {/* The buttons wrap below the text when the chat is narrow, so "Question N of M" stays readable. */}
        <div className="ml-auto flex shrink-0 items-center gap-2">
        {next && (
          <button className="btn shrink-0" onClick={jump} aria-label="Jump to questions" title={`Go to question ${next.n}, the first without an answer`}>
            <ArrowDownToLine className="size-3" /> <span className="max-md:hidden">Jump to questions</span>
          </button>
        )}
        {session.round != null && !session.waiting && (
          <ActionButton
            variant="secondary"
            className="shrink-0"
            icon={<Send className="size-3" />}
            pending={busy && command === "send"}
            pendingLabel={<span className="max-md:hidden">Sending…</span>}
            onClick={() => send("send")}
            disabled={busy}
            aria-label="Send round"
            title="Send your answers to the agent"
          >
            <span className="max-md:hidden">Send round</span>
          </ActionButton>
        )}
        <ActionButton
          variant="secondary"
          className="shrink-0"
          icon={paused ? <Play className="size-3 fill-current" /> : <Pause className="size-3 fill-current" />}
          pending={busy && (command === "pause" || command === "resume")}
          pendingLabel={<span className="max-md:hidden">{command === "resume" ? "Resuming…" : "Pausing…"}</span>}
          onClick={() => send(paused ? "resume" : "pause")}
          disabled={busy}
          aria-label={paused ? "Resume" : "Pause"}
          title={paused ? "Resume" : "Pause"}
        >
          <span className="max-md:hidden">{paused ? "Resume" : "Pause"}</span>
        </ActionButton>
        <button className="btn shrink-0" onClick={() => setConfirming(true)} disabled={busy} aria-label="End interview" title="End interview">
          <Square className="size-3 fill-current" /> <span className="max-md:hidden">End interview</span>
        </button>
        {!fullScreen && (
          <button className="btn-ghost shrink-0" onClick={() => navigate(interviewPath(session.agent_id))} aria-label="Full screen" title="Answer on a full page">
            <Maximize2 className="size-3" />
          </button>
        )}
        </div>
      </div>
      <ErrorLine error={commanding.error && grillError(commanding.error)} className="-mt-2 mb-2" />

      {confirming && (
        <ConfirmDialog
          title="End the interview?"
          body="The rounds stop now. What you sent is kept, and the agent writes a summary of what was covered."
          cancelLabel="Keep going"
          confirmLabel="End interview"
          pendingLabel="Ending…"
          danger
          pending={busy}
          onConfirm={() => send("end")}
          onClose={() => setConfirming(false)}
        />
      )}
    </>
  );
}
