import { createContext, useContext, useMemo, useState } from "react";
import { ChevronRight, GitMerge, GitPullRequest } from "lucide-react";
import { api, type PullRequest } from "../../lib/api";
import { cx } from "../../lib/format";
import { mergeOrder, namedPrs, type MergeRow } from "../../lib/prRefs";
import { useAction } from "../../lib/useAction";
import { ActionButton } from "../../components/ActionButton";
import { Checks, PullRequestCard, StateBadge, Verdicts, mergeability, useCatchUp } from "./PullRequestCard";

/** Every tracked pull request, live, for the messages that name them. */
export const PrsContext = createContext<PullRequest[]>([]);

/** The cards for the tracked pull requests `text` names (see namedPrs); nothing when it names none. */
export function MessagePrs({ text, project, className }: { text: string; project: string; className?: string }) {
  const all = useContext(PrsContext);
  const prs = useMemo(() => namedPrs(text, project.trim(), all), [text, project, all]);
  return prs.length ? <div className={className}><TaggedPrs prs={prs} /></div> : null;
}

/** The pull requests a message names: one PR's full card, or several as one merge-order card. */
export function TaggedPrs({ prs }: { prs: PullRequest[] }) {
  if (!prs.length) return null;
  if (prs.length === 1) return <PullRequestCard pr={prs[0]} />;
  return (
    <div className="overflow-hidden rounded-2xl border border-line-strong bg-ink-900/80">
      <div className="flex items-center gap-2 px-4 py-2.5 text-[13px] font-medium">
        <GitPullRequest className="size-3.5 text-fg-3" /> Pull requests
        <span className="font-mono text-[11px] font-normal text-fg-3">{prs.length}</span>
      </div>
      <div className="divide-y divide-line border-t border-line">
        {mergeOrder(prs).map((r) => <Row key={r.pr.id} row={r} />)}
      </div>
    </div>
  );
}

/** One PR of the merge order: number, title, status and Merge; it opens to the full card. */
function Row({ row: { pr, after } }: { row: MergeRow }) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const merging = useAction();
  const m = mergeability(pr, after);
  const method = pr.merge_methods[0] ?? "squash";
  useCatchUp(pr);

  const merge = () => merging.run(async () => {
    await api.mergePullRequest(pr.id, method);
    setConfirming(false);
  });

  return (
    <div>
      <div className="px-4 py-2.5">
        <button
          className="flex w-full min-w-0 items-center gap-2 text-left"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-label={`${open ? "Hide" : "Show"} pull request #${pr.number}`}
        >
          <ChevronRight className={cx("size-3.5 shrink-0 text-fg-3 transition", open && "rotate-90")} />
          <span className="shrink-0"><StateBadge pr={pr} /></span>
          <span className="shrink-0 font-mono text-[11px] text-fg-3">#{pr.number}</span>
          <span className="min-w-0 truncate text-[13px]">{pr.title || `Pull request #${pr.number}`}</span>
        </button>
        <div className="mt-1.5 flex flex-wrap items-center gap-2.5 pl-[22px]">
          {after && <span className="font-mono text-[11px] text-fg-3">after #{after}</span>}
          {pr.state === "open" && <Checks checks={pr.checks} />}
          {pr.state === "open" && <Verdicts labels={pr.labels} />}
          {pr.state === "open" && (confirming ? (
            <span className="ml-auto flex items-center gap-1.5">
              <button className="btn-ghost" onClick={() => setConfirming(false)} disabled={merging.busy}>Cancel</button>
              <ActionButton
                variant="bare"
                className="gap-1.5 rounded-full bg-merged px-3 py-1 text-[12.5px] font-medium text-ink-950 transition hover:brightness-110"
                onClick={merge}
                pending={merging.busy}
                pendingLabel="Merging…"
                icon={<GitMerge className="size-3.5" />}
              >
                Confirm {method}
              </ActionButton>
            </span>
          ) : (
            <button
              className="ml-auto inline-flex h-[26px] items-center gap-1.5 rounded-full bg-accent px-3 text-[12.5px] font-medium text-ink-950 transition hover:brightness-110 disabled:bg-ink-700 disabled:text-fg-3"
              onClick={() => setConfirming(true)}
              disabled={!m.canMerge}
              title={m.canMerge ? undefined : m.label}
            >
              <GitMerge className="size-3.5" /> Merge
            </button>
          ))}
        </div>
        {merging.error && <div className="mt-1 pl-[22px] text-[11.5px] text-bad">{merging.error}</div>}
      </div>
      {open && <div className="px-3 pb-3"><PullRequestCard pr={pr} after={after} /></div>}
    </div>
  );
}
