import { useEffect, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, CornerDownLeft, MessageCircle, Send, Sparkles, Wand2 } from "lucide-react";
import { api, type GrillQuestion, type GrillSession, type Message } from "../../lib/api";
import { cx } from "../../lib/format";

type Posted = Pick<GrillQuestion, "id" | "text" | "recommendation" | "from_round">;

/**
 * A round the agent posted (`meta.grill.role == "round"`). The round waiting for answers shows as cards
 * with Accept and an answer field, fed by the live session; earlier rounds show their questions read-only.
 */
export function GrillRound({ m, agentId, session }: { m: Message; agentId: string; session: GrillSession | null }) {
  const g = m.meta?.grill ?? {};
  const open = !!session && session.session_id === g.session_id && session.round === g.round && !session.waiting && session.state !== "ended";
  const posted: Posted[] = g.questions ?? [];
  const live = open ? (session?.questions ?? []) : [];
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sendRound = async () => {
    if (!session) return;
    setBusy(true);
    setError(null);
    try {
      await api.grillCommand(session.session_id, "send");
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };

  const answered = live.filter((q) => q.state !== "open").length;
  return (
    <div className="pl-9">
      <div className={cx("rounded-2xl border p-4", open ? "border-accent/25 bg-accent/[0.04]" : "border-line bg-ink-900/60")}>
        <div className="mb-3 flex items-center gap-2 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">
          <span className={open ? "text-accent" : undefined}>Round {g.round}</span>
          {g.topic && <span className="truncate normal-case tracking-normal">· {g.topic}</span>}
        </div>
        <div className="space-y-3">
          {open
            ? live.map((q, i) => <QuestionCard key={q.id} n={i + 1} q={q} agentId={agentId} sessionId={session!.session_id} />)
            : posted.map((q, i) => (
                <div key={q.id} className="text-[13.5px]">
                  <div className="prose-chat text-fg-2">
                    <Markdown remarkPlugins={[remarkGfm]}>{`${i + 1}. ${q.text}`}</Markdown>
                  </div>
                  <div className="mt-0.5 pl-4 text-[12.5px] text-fg-3">Recommended: {q.recommendation}</div>
                </div>
              ))}
        </div>
        {open && (
          <div className="mt-4 flex items-center gap-3 border-t border-line pt-3">
            <span className="flex-1 font-mono text-[11px] text-fg-3">
              {answered} of {live.length} answered{answered < live.length ? " · unanswered ones come back next round" : ""}
            </span>
            <button className="btn-primary" disabled={busy} onClick={sendRound}>
              <Send className="size-3.5" /> Send round
            </button>
          </div>
        )}
        {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
      </div>
    </div>
  );
}

function message(e: unknown) {
  const m = (e as Error).message;
  if (m === "grill_closed") return "This interview has already ended.";
  if (m === "round_closed" || m === "question_closed") return "This round was already sent.";
  return m;
}

/** One question of the open round: the recommendation with Accept or Decide for me, and a field for any
 * other answer, saved as a decision or as a comment to discuss. */
function QuestionCard({ n, q, agentId, sessionId }: { n: number; q: GrillQuestion; agentId: string; sessionId: string }) {
  const written = (q.state === "answered" && !q.accepted) || q.state === "discuss";
  const [draft, setDraft] = useState(written ? (q.answer ?? "") : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Answers given elsewhere (the app) show here too.
  useEffect(() => {
    if (written) setDraft(q.answer ?? "");
  }, [written, q.answer]);

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };
  const answer = (text: string, discuss = false) => {
    if (text.trim()) run(() => api.grillAnswer(agentId, sessionId, q.id, text.trim(), discuss));
  };

  const same = written && q.answer === draft.trim();
  return (
    <div className="rounded-xl border border-line bg-ink-900/70 p-3">
      {q.from_round != null && <div className="mb-1 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">Still open from round {q.from_round}</div>}
      <div className="prose-chat text-[14px]">
        <Markdown remarkPlugins={[remarkGfm]}>{`${n}. ${q.text}`}</Markdown>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <div className="flex min-w-[14rem] flex-1 items-start gap-1.5 text-[13px] text-fg-2">
          <Sparkles className="mt-0.5 size-3.5 shrink-0 text-accent" />
          <span>
            <span className="text-fg-3">Recommended:</span> {q.recommendation}
          </span>
        </div>
        {q.accepted ? (
          <span className="inline-flex items-center gap-1 font-mono text-[11.5px] text-good">
            <Check className="size-3.5" /> Accepted
          </span>
        ) : (
          <button className="btn shrink-0" disabled={busy} onClick={() => answer("Yes")}>
            <Check className="size-3.5" /> Accept
          </button>
        )}
        {q.state === "decided" ? (
          <span className="inline-flex items-center gap-1 font-mono text-[11.5px] text-good">
            <Wand2 className="size-3.5" /> Left to the agent
          </span>
        ) : (
          <button className="btn-ghost shrink-0" disabled={busy} onClick={() => run(() => api.grillCommand(sessionId, "decide", q.id))} title="The agent's recommendation becomes the decision">
            <Wand2 className="size-3.5" /> Decide for me
          </button>
        )}
      </div>
      <form
        className="mt-2 flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          answer(draft);
        }}
      >
        <input
          className="input py-1.5"
          placeholder={q.accepted || q.state === "decided" ? "Or answer differently…" : "Your answer, or a comment to discuss…"}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          aria-label={`Answer to question ${n}`}
        />
        {same && q.state === "answered" ? (
          <span className="inline-flex shrink-0 items-center gap-1 font-mono text-[11.5px] text-good">
            <Check className="size-3.5" /> Saved
          </span>
        ) : (
          <button className="btn shrink-0" type="submit" disabled={busy || !draft.trim()} aria-label="Save answer" title="Save as your answer">
            <CornerDownLeft className="size-3.5" />
          </button>
        )}
        {same && q.state === "discuss" ? (
          <span className="inline-flex shrink-0 items-center gap-1 font-mono text-[11.5px] text-accent">
            <MessageCircle className="size-3.5" /> To discuss
          </span>
        ) : (
          <button
            className="btn shrink-0"
            type="button"
            disabled={busy || !draft.trim()}
            onClick={() => answer(draft, true)}
            title="Not a decision: the agent explains and asks again next round"
          >
            <MessageCircle className="size-3.5" /> <span className="max-md:hidden">Discuss</span>
          </button>
        )}
      </form>
      {error && <div className="mt-1.5 text-[12px] text-bad">{error}</div>}
    </div>
  );
}
