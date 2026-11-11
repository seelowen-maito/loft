import { useEffect, useRef } from "react";
import { useNavigate } from "react-router";
import { ArrowLeft, Hourglass } from "lucide-react";
import type { Agent, GrillSession } from "../../lib/api";
import { interviewShows } from "../../lib/interview";
import { GrillBar } from "./GrillBar";
import { QuestionCard } from "./GrillRound";

/**
 * The open interview on its own page, over everything: the round bar (progress, Send round, Pause, End) and the
 * round's question cards, the same cards the chat shows, so answer Drafts are shared with it. Esc or Back to chat
 * returns; it returns by itself when the interview ends, like the app's.
 */
export function InterviewView({ agent, session }: { agent: Agent; session: GrillSession | null }) {
  const navigate = useNavigate();
  const shows = interviewShows(session);
  const back = () => navigate(`/agents/${agent.id}`);
  const backRef = useRef(back);
  backRef.current = back;

  useEffect(() => {
    // A handler that already used Esc (the End confirmation, the @ menu) keeps it.
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !e.defaultPrevented && !e.isComposing && backRef.current();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Only an interview seen open here closes the page: on a direct load the session is still on its way.
  const seen = useRef(false);
  useEffect(() => {
    if (shows !== "none") seen.current = true;
    else if (seen.current) backRef.current();
  }, [shows]);

  return (
    <div className="fixed inset-0 z-50 overflow-y-auto bg-ink-950" role="dialog" aria-modal aria-label="Interview">
      {/* Opaque, no backdrop blur: a filter here would trap the End confirmation's fixed overlay inside the header. */}
      <div className="sticky top-0 z-10 border-b border-line bg-ink-950">
        <div className="mx-auto flex max-w-3xl items-center gap-3 px-4 py-3">
          <button className="btn-ghost shrink-0" onClick={back} title="Back to chat (Esc)">
            <ArrowLeft className="size-3.5" /> Back to chat
          </button>
          <h1 className="min-w-0 flex-1 truncate text-right text-[13px] text-fg-2">Interview with {agent.name}</h1>
        </div>
        {session && shows !== "none" && (
          <div className="mx-auto max-w-3xl px-4">
            <GrillBar session={session} fullScreen />
          </div>
        )}
      </div>

      <div className="mx-auto max-w-3xl space-y-3 px-4 py-4">
        {shows === "round" && session ? (
          (session.questions ?? []).map((q, i) => (
            <QuestionCard key={q.id} n={i + 1} q={q} agentId={agent.id} sessionId={session.session_id} round={session.round!} />
          ))
        ) : shows === "waiting" ? (
          <div className="flex items-center gap-2 py-16 text-[15px] text-fg-3" role="status">
            <Hourglass className="size-4" /> {session?.round == null ? "Preparing questions…" : "Next round…"}
          </div>
        ) : (
          <div className="space-y-3 py-16 text-[15px] text-fg-3">
            <p>No interview is open with {agent.name}.</p>
            <button className="btn" onClick={back}>
              <ArrowLeft className="size-3.5" /> Back to chat
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
