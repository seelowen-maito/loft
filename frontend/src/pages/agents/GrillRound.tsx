import { useEffect, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, CornerDownLeft, Send, Sparkles } from "lucide-react";
import { api, type GrillQuestion, type GrillSession, type Message } from "../../lib/api";
import { cx } from "../../lib/format";

type Posted = Pick<GrillQuestion, "id" | "text" | "recommendation">;

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
              {answered} of {live.length} answered{answered < live.length ? " · unanswered questions are sent as open" : ""}
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

/** One question of the open round: the recommendation with Accept, and a field for any other answer. */
function QuestionCard({ n, q, agentId, sessionId }: { n: number; q: GrillQuestion; agentId: string; sessionId: string }) {
  const [draft, setDraft] = useState(q.state === "answered" && !q.accepted ? (q.answer ?? "") : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Answers given elsewhere (typed in the composer, the app) show here too.
  useEffect(() => {
    if (q.state === "answered" && !q.accepted) setDraft(q.answer ?? "");
  }, [q.state, q.accepted, q.answer]);

  const answer = async (text: string) => {
    if (!text.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await api.grillAnswer(agentId, sessionId, q.id, text.trim());
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };

  const saved = q.state === "answered" && !q.accepted && q.answer === draft.trim();
  return (
    <div className="rounded-xl border border-line bg-ink-900/70 p-3">
      <div className="prose-chat text-[14px]">
        <Markdown remarkPlugins={[remarkGfm]}>{`${n}. ${q.text}`}</Markdown>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <div className="flex min-w-0 flex-1 items-start gap-1.5 text-[13px] text-fg-2">
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
      </div>
      {q.state === "decided" && <div className="mt-2 font-mono text-[11.5px] text-fg-3">Left to the agent: its recommendation stands.</div>}
      <form
        className="mt-2 flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          answer(draft);
        }}
      >
        <input
          className="input py-1.5"
          placeholder={q.accepted ? "Or answer differently…" : "Your answer…"}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          aria-label={`Answer to question ${n}`}
        />
        {saved ? (
          <span className="inline-flex shrink-0 items-center gap-1 font-mono text-[11.5px] text-good">
            <Check className="size-3.5" /> Saved
          </span>
        ) : (
          <button className="btn shrink-0" type="submit" disabled={busy || !draft.trim()} aria-label="Save answer">
            <CornerDownLeft className="size-3.5" />
          </button>
        )}
      </form>
      {error && <div className="mt-1.5 text-[12px] text-bad">{error}</div>}
    </div>
  );
}
