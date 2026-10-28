import { useState } from "react";
import { Link } from "react-router";
import { CircleDot, Eraser, ExternalLink, FileText, Loader2, Users } from "lucide-react";
import { api, type EndCard as Card, type PlannedIssue } from "../../lib/api";

const ROLE: Record<string, string> = { dev: "Dev", reviewer: "reviewer" };

/** A published spec or ticket: a GitHub issue, or (no repo) a Loft issue, which opens here and has no number. */
function Issue({ i, icon, inLoft }: { i: PlannedIssue; icon: React.ReactNode; inLoft: boolean }) {
  const cls = "group flex items-start gap-2.5 rounded-lg px-2 py-1.5 hover:bg-white/[0.04]";
  const inner = (
    <>
      {icon}
      <span className="min-w-0 flex-1 text-[13px]">
        {!inLoft && <span className="font-mono text-fg-3">#{i.number}</span>} {i.title || "Untitled"}
      </span>
      {!inLoft && <ExternalLink className="mt-0.5 size-3.5 shrink-0 text-fg-3 opacity-0 transition group-hover:opacity-100" />}
    </>
  );
  return inLoft ? <Link to={i.url} className={cls}>{inner}</Link> : <a href={i.url} target="_blank" rel="noopener noreferrer" className={cls}>{inner}</a>;
}

/** The end of a planning grill: the spec and tickets it published, Clear context with the next message, and the team check. */
export function EndCard({ agentId, messageId, card, onClear }: { agentId: string; messageId: number; card: Card; onClear: (next: string) => Promise<void> }) {
  const [busy, setBusy] = useState<"clear" | "setup" | null>(null);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = async (what: "clear" | "setup", f: () => Promise<unknown>) => {
    setBusy(what);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(null);
    }
  };
  const setup = card.setup;
  const shown = sent || !!setup?.sent_at;
  return (
    <div className="ml-9 rounded-2xl border border-line-strong bg-ink-850 p-4">
      {/* No repo: the plan was kept as Loft issues. */}
      <div className="font-mono text-[10.5px] uppercase tracking-wider text-fg-3">Planning done · {card.repo || "kept in Loft"}</div>
      <div className="mt-2 space-y-0.5">
        {card.spec && <Issue i={card.spec} inLoft={!card.repo} icon={<FileText className="mt-0.5 size-3.5 shrink-0 text-accent" />} />}
        {card.tickets.map((t) => <Issue key={t.url} i={t} inLoft={!card.repo} icon={<CircleDot className="mt-0.5 size-3.5 shrink-0 text-good" />} />)}
      </div>
      {setup && (
        <div className="mt-3 rounded-xl border border-warn/25 bg-warn/[0.06] px-3 py-2 text-[12.5px] text-fg-2">
          The team has no {card.missing_roles.map((r) => ROLE[r] ?? r).join(" or ")} yet.
        </div>
      )}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <button
          className="btn btn-primary text-[12.5px]"
          disabled={busy !== null}
          title={`Starts a fresh session and leaves "${card.next_message}" in the composer.`}
          onClick={() => run("clear", () => onClear(card.next_message))}
        >
          {busy === "clear" ? <Loader2 className="size-3.5 animate-spin" /> : <Eraser className="size-3.5" />} Clear context
        </button>
        {setup && !shown && (
          <button
            className="btn text-[12.5px]"
            disabled={busy !== null}
            title={setup.message}
            onClick={() => run("setup", async () => {
              await api.sendTeamSetup(agentId, messageId);
              setSent(true);
            })}
          >
            {busy === "setup" ? <Loader2 className="size-3.5 animate-spin" /> : <Users className="size-3.5" />} Set up the team with lofty
          </button>
        )}
        {setup && shown && (
          <Link to={`/agents/${setup.agent_id}`} className="font-mono text-[11.5px] text-accent hover:underline">
            Sent to lofty · open its chat
          </Link>
        )}
      </div>
      <div className="mt-2 font-mono text-[10.5px] text-fg-3">Next: {card.next_message}</div>
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
    </div>
  );
}
