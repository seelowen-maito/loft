import { useEffect, useState } from "react";
import { Link } from "react-router";
import { CircleDot, Eraser, ExternalLink, FileText, GitBranch, GitMerge, Loader2, Users } from "lucide-react";
import { cx } from "../../lib/format";
import { teamNote } from "../../lib/teamNote";
import { StartDeliveryButton } from "../../components/StartDeliveryButton";
import { StartWarning } from "../../components/StartWarning";
import { api, type WaitsOn, type DeliveryStarted, type EndCard as Card, type CardSpec, type PlannedIssue } from "../../lib/api";

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

/** A spec's key on the card: two repos can each have a #1. */
const keyOf = (d: { repo: string; spec: number }) => `${d.repo}#${d.spec}`;

/** Who started this spec, read from the card the backend sends back. */
function startedOf(card: Card | undefined, d: { repo: string; spec: number }): DeliveryStarted | null {
  return [card, ...(card?.more ?? [])].find((c) => c?.delivery && keyOf(c.delivery) === keyOf(d))?.delivery?.started ?? null;
}

type Run = (what: "clear" | "setup" | "start" | "merge", f: () => Promise<unknown>) => Promise<void>;

/** One published spec (or standalone ticket) with its tickets and its own Start delivery. */
function SpecBlock({ agentId, messageId, item, busy, run, outcome, showRepo, note, extra }: { agentId: string; messageId: number; item: CardSpec; busy: string | null; run: Run; outcome?: DeliveryStarted | string; showRepo: boolean; note?: React.ReactNode; extra?: React.ReactNode }) {
  const [started, setStarted] = useState<DeliveryStarted | null>(null);
  // A plan kept in a folder with no git: Start delivery asks before setting it up.
  const [askGit, setAskGit] = useState<string | null>(null);
  const [declined, setDeclined] = useState(false);
  const delivery = item.delivery;
  // What the spec's tickets wait on in another spec, read while it hasn't started: Start delivery warns first.
  const [waits, setWaits] = useState<WaitsOn[]>([]);
  const [warn, setWarn] = useState(false);
  const notStarted = !!delivery && !delivery.started && !started;
  const reloadWaits = () => {
    if (delivery) api.deliverySummaries().then((all) => setWaits(all.find((d) => d.repo === delivery.repo && d.spec === delivery.spec)?.waits_on ?? [])).catch(() => {});
  };
  useEffect(reloadWaits, [delivery?.repo, delivery?.spec, notStarted]);
  const start = (initGit: boolean) =>
    run("start", async () => {
      setWarn(false);
      if (!delivery) return;
      const r = await api.startDelivery(agentId, messageId, delivery.repo, delivery.spec, initGit);
      if (r.needs_git) return setAskGit(r.dir ?? "the project folder");
      setAskGit(null);
      setStarted(startedOf(r.message.meta?.grill?.end_card, delivery));
    });
  const startedOn = started ?? (typeof outcome === "object" ? outcome : null) ?? delivery?.started ?? null;
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
    <div>
      {showRepo && <div className="mb-1 font-mono text-[10.5px] text-fg-3">{item.repo || "kept in Loft"}</div>}
      <div className="space-y-0.5">
        {item.spec && <Issue i={item.spec} inLoft={!item.repo} icon={<FileText className="mt-0.5 size-3.5 shrink-0 text-accent" />} />}
        {item.tickets.map((t) => <Issue key={t.url} i={t} inLoft={!item.repo} icon={<CircleDot className="mt-0.5 size-3.5 shrink-0 text-good" />} />)}
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
      {note}
      {warn && !startedOn && waits.length > 0 && <StartWarning waits={waits} onStart={() => start(false)} onCancel={() => setWarn(false)} onStarted={reloadWaits} />}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {delivery && !startedOn && !askGit && (
          <StartDeliveryButton
            className="text-[12.5px]"
            pending={busy === "start"}
            disabled={busy !== null}
            title={delivery.standalone ? `Opens #${delivery.spec}'s branch and draft PR and starts it on the team's Dev.` : `Starts spec #${delivery.spec}'s first ready ticket on the team's Dev.`}
            onClick={() => (waits.length ? setWarn(true) : start(false))}
          />
        )}
        {delivery && startedOn && (
          <Link to={`/agents/${startedOn.agent_id}`} className="font-mono text-[11.5px] text-accent hover:underline">
            {startedOn.ticket === 0 ? `Delivery started · ${startedOn.agent_name} hands out the tickets` : `Delivery started · ${delivery.local ? `“${startedOn.title ?? ""}”` : `#${startedOn.ticket}`} with ${startedOn.agent_name}`}
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
        {typeof outcome === "string" && !startedOn && <span className="text-[12px] text-bad">{outcome}</span>}
        {delivery?.local && (merged || stage?.state === "merged") && <span className="font-mono text-[11.5px] text-good">{merged ?? `Merged ${stage?.branch} into ${stage?.base}`}</span>}
        {extra}
      </div>
    </div>
  );
}

/** The end of a planning grill: the specs and tickets it published, each with Start delivery when it has a
 *  Delivery (otherwise Clear context leaves the next message in the composer), and the team check. */
export function EndCard({ agentId, messageId, card, onClear }: { agentId: string; messageId: number; card: Card; onClear: (next: string) => Promise<void> }) {
  const [busy, setBusy] = useState<"clear" | "setup" | "start" | "merge" | null>(null);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run: Run = async (what, f) => {
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
  const items: CardSpec[] = [card, ...(card.more ?? [])];
  // Start every spec: one after the other, each on its own Lead; a spec that can't start says why, the rest go on.
  const [outcome, setOutcome] = useState<Record<string, DeliveryStarted | string>>({});
  const todo = items.filter((i) => i.delivery && !i.delivery.started && typeof outcome[keyOf(i.delivery)] !== "object");
  const startAll = () =>
    run("start", async () => {
      for (const { delivery } of todo) {
        if (!delivery) continue;
        let result: DeliveryStarted | string;
        try {
          const r = await api.startDelivery(agentId, messageId, delivery.repo, delivery.spec);
          result = r.needs_git ? "Has no git yet: start it on its own." : (startedOf(r.message.meta?.grill?.end_card, delivery) ?? "Didn't start.");
        } catch (e) {
          result = (e as Error).message;
        }
        setOutcome((o) => ({ ...o, [keyOf(delivery)]: result }));
      }
    });
  const teamLine = setup && (
    <div className="mt-3 flex items-start gap-2 text-[12.5px] text-fg-2">
      <Users className="mt-0.5 size-3.5 shrink-0 text-fg-3" />
      {teamNote(card.missing_roles)}
    </div>
  );
  const actions = (
    <>
      <button
        className={cx("btn text-[12.5px]", items.some((i) => !i.delivery) && "btn-primary")}
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
    </>
  );
  return (
    <div className="ml-9 rounded-2xl border border-line-strong bg-ink-850 p-4">
      {/* No repo: the plan was kept as Loft issues. */}
      <div className="font-mono text-[10.5px] uppercase tracking-wider text-fg-3">Planning done{items.length === 1 && ` · ${card.repo || "kept in Loft"}`}</div>
      <div className="mt-2 space-y-3 divide-y divide-line [&>*:not(:first-child)]:pt-3">
        {items.map((item, i) => <SpecBlock key={item.delivery ? keyOf(item.delivery) : (item.spec?.url ?? i)} agentId={agentId} messageId={messageId} item={item} busy={busy} run={run} outcome={item.delivery ? outcome[keyOf(item.delivery)] : undefined} showRepo={items.length > 1} note={items.length === 1 ? teamLine : undefined} extra={items.length === 1 ? actions : undefined} />)}
      </div>
      {items.length > 1 && teamLine}
      {items.length > 1 && (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          {todo.length > 0 && (
            <StartDeliveryButton
              className="text-[12.5px]"
              label={todo.length === items.length ? `Start all ${todo.length}` : `Start remaining ${todo.length}`}
              pending={busy === "start"}
              disabled={busy !== null}
              title={todo.length === 1 ? "Starts the spec that isn't started yet, on its own Lead." : `Starts the ${todo.length} specs that aren't started yet, each on its own Lead.`}
              onClick={startAll}
            />
          )}
          {actions}
        </div>
      )}
      {card.next_message && <div className="mt-2 font-mono text-[10.5px] text-fg-3">Next: {card.next_message}</div>}
      {error && <div className="mt-2 text-[12px] text-bad">{error}</div>}
    </div>
  );
}
