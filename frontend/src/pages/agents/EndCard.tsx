import { useEffect, useState } from "react";
import { Link } from "react-router";
import { CircleDot, Eraser, ExternalLink, FileText, GitBranch, GitMerge, Loader2, Users } from "lucide-react";
import { cx } from "../../lib/format";
import { teamNote } from "../../lib/teamNote";
import { StartDeliveryButton } from "../../components/StartDeliveryButton";
import { api, type DeliveryStarted, type EndCard as Card, type PlannedIssue } from "../../lib/api";

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

/** The end of a planning grill: the spec and tickets it published, Start delivery for a spec with a Delivery
 *  (otherwise Clear context leaves the next message in the composer), and the team check. */
export function EndCard({ agentId, messageId, card, onClear }: { agentId: string; messageId: number; card: Card; onClear: (next: string) => Promise<void> }) {
  const [busy, setBusy] = useState<"clear" | "setup" | "start" | "merge" | null>(null);
  const [sent, setSent] = useState(false);
  const [started, setStarted] = useState<DeliveryStarted | null>(null);
  // A plan kept in a folder with no git: Start delivery asks before setting it up.
  const [askGit, setAskGit] = useState<string | null>(null);
  const [declined, setDeclined] = useState(false);
  const start = (initGit: boolean) =>
    run("start", async () => {
      const r = await api.startDelivery(agentId, messageId, initGit);
      if (r.needs_git) return setAskGit(r.dir ?? "the project folder");
      setAskGit(null);
      setStarted(r.message.meta?.grill?.end_card?.delivery?.started ?? null);
    });
  const [error, setError] = useState<string | null>(null);
  const run = async (what: "clear" | "setup" | "start" | "merge", f: () => Promise<unknown>) => {
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
  const delivery = card.delivery;
  const startedOn = started ?? delivery?.started ?? null;
  // A plan kept in Loft ends with Merge, once its reviews passed.
  const [stage, setStage] = useState<{ state: string; branch: string; base: string } | null>(null);
  const [merged, setMerged] = useState<string | null>(null);
  useEffect(() => {
    if (!delivery?.local || !startedOn) return;
    let live = true;
    api.deliveryState(delivery.repo, delivery.spec).then((s) => live && setStage(s)).catch(() => {});
    return () => {
      live = false;
    };
  }, [delivery?.local, delivery?.repo, delivery?.spec, !!startedOn]);
  return (
    <div className="ml-9 rounded-2xl border border-line-strong bg-ink-850 p-4">
      {/* No repo: the plan was kept as Loft issues. */}
      <div className="font-mono text-[10.5px] uppercase tracking-wider text-fg-3">Planning done · {card.repo || "kept in Loft"}</div>
      <div className="mt-2 space-y-0.5">
        {card.spec && <Issue i={card.spec} inLoft={!card.repo} icon={<FileText className="mt-0.5 size-3.5 shrink-0 text-accent" />} />}
        {card.tickets.map((t) => <Issue key={t.url} i={t} inLoft={!card.repo} icon={<CircleDot className="mt-0.5 size-3.5 shrink-0 text-good" />} />)}
      </div>
      {askGit && (
        <div className="mt-3 space-y-2 rounded-xl border border-accent/25 bg-accent/[0.05] px-3 py-2 text-[12.5px] text-fg-2">
          <div>
            <span className="break-all font-mono text-[11.5px] text-fg">{askGit}</span> has no git yet. Set it up? Loft runs <span className="font-mono">git init</span>, adds a <span className="font-mono">.gitignore</span> for junk and secret files like <span className="font-mono">.env</span> and keys, and commits the folder as it is. Then the Delivery starts.
          </div>
          <div className="flex flex-wrap gap-2">
            <button className="btn btn-primary text-[12px]" disabled={busy !== null} onClick={() => start(true)}>
              {busy === "start" ? <Loader2 className="size-3.5 animate-spin" /> : <GitBranch className="size-3.5" />} Set up git and start
            </button>
            <button className="btn text-[12px]" disabled={busy !== null} onClick={() => { setAskGit(null); setDeclined(true); }}>
              Not now
            </button>
          </div>
        </div>
      )}
      {declined && !askGit && !startedOn && <div className="mt-3 text-[12px] text-fg-3">Nothing was set up: the plan stays as Loft issues.</div>}
      {setup && (
        <div className="mt-3 flex items-start gap-2 text-[12.5px] text-fg-2">
          <Users className="mt-0.5 size-3.5 shrink-0 text-fg-3" />
          {teamNote(card.missing_roles)}
        </div>
      )}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {delivery && !startedOn && !askGit && (
          <StartDeliveryButton
            className="text-[12.5px]"
            pending={busy === "start"}
            disabled={busy !== null}
            title={delivery.standalone ? `Opens #${delivery.spec}'s branch and draft PR and starts it on the team's Dev.` : `Starts spec #${delivery.spec}'s first ready ticket on the team's Dev.`}
            onClick={() => start(false)}
          />
        )}
        {delivery && startedOn && (
          <Link to={`/agents/${startedOn.agent_id}`} className="font-mono text-[11.5px] text-accent hover:underline">
            Delivery started · {delivery.local ? `“${startedOn.title ?? ""}”` : `#${startedOn.ticket}`} with {startedOn.agent_name}
          </Link>
        )}
        {delivery?.local && stage?.state === "ready" && !merged && (
          <button
            className="btn btn-primary text-[12.5px]"
            disabled={busy !== null}
            title={`Merges ${stage.branch} into ${stage.base} in the Dev's clone. Your copy of the project isn't touched.`}
            onClick={() => run("merge", async () => setMerged((await api.mergeDelivery(delivery.repo, delivery.spec)).message))}
          >
            {busy === "merge" ? <Loader2 className="size-3.5 animate-spin" /> : <GitMerge className="size-3.5" />} Merge
          </button>
        )}
        {delivery?.local && (merged || stage?.state === "merged") && <span className="font-mono text-[11.5px] text-good">{merged ?? `Merged ${stage?.branch} into ${stage?.base}`}</span>}
        <button
          className={cx("btn text-[12.5px]", !delivery && "btn-primary")}
          disabled={busy !== null}
          title={card.next_message ? `Starts a fresh session and leaves "${card.next_message}" in the composer.` : "Starts a fresh session."}
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
      {card.next_message && <div className="mt-2 font-mono text-[10.5px] text-fg-3">Next: {card.next_message}</div>}
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
    </div>
  );
}
