import { MessageCircleQuestion, Reply } from "lucide-react";
import type { AnswerCard as Card } from "../../lib/answerCard";

/** Above the composer when the agent's latest message asks you something: its question, one-click answers (Yes/No,
 * or the options it listed) and Reply…, which quotes it in the composer. The question is plain text, as quotes are. */
export function AnswerCard({ card, agentName, busy, onAnswer, onReply }: {
  card: Card;
  agentName: string;
  busy: boolean;
  onAnswer: (text: string) => void;
  onReply: () => void;
}) {
  return (
    <div className="mb-3 rounded-xl border border-accent/30 bg-accent/[0.06] px-3 py-2.5" role="group" aria-label={`${agentName} asks`}>
      <div className="flex items-start gap-2">
        <MessageCircleQuestion className="mt-0.5 size-4 shrink-0 text-accent" aria-hidden />
        <div className="min-w-0 flex-1">
          <div className="font-mono text-[10.5px] uppercase tracking-wider text-fg-3">{agentName} asks</div>
          <div className="line-clamp-3 [overflow-wrap:anywhere] text-[13px] text-fg">{card.question}</div>
        </div>
      </div>
      <div className="mt-2 flex flex-wrap gap-2 pl-6">
        {card.answers.map((a) => (
          <button key={a} type="button" className="btn max-w-full text-[12.5px]" disabled={busy} onClick={() => onAnswer(a)} title={`Send “${a}”`}>
            <span className="truncate">{a}</span>
          </button>
        ))}
        <button type="button" className="btn-ghost text-[12.5px] text-fg-2" onClick={onReply}>
          <Reply className="size-3.5" aria-hidden /> Reply…
        </button>
      </div>
    </div>
  );
}
