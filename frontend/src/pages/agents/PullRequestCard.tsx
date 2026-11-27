import { useEffect, useState } from "react";
import {
  AlertTriangle, ArrowRight, Check, CircleDashed, ExternalLink, GitMerge, GitPullRequest, GitPullRequestClosed,
  GitPullRequestDraft, Loader2, RefreshCw, ShieldAlert, X,
} from "lucide-react";
import { api, type PullRequest } from "../../lib/api";
import { cx } from "../../lib/format";
import { ActionButton } from "../../components/ActionButton";
import { Callout } from "../../components/Callout";
import { relTime } from "../../lib/time";
import { verdicts } from "../../lib/verdicts";
import { mergeability, type MergeIcon, type Tone } from "../../lib/mergeability";
import { useAction } from "../../lib/useAction";

const METHOD_LABEL: Record<string, string> = { squash: "Squash and merge", merge: "Create merge commit", rebase: "Rebase and merge" };

/** The method a PR's Merge uses: its project's merge method, as Loft last read it, which the server merges with. */
export function mergeMethod(pr: PullRequest): string {
  return pr.merge_method || pr.merge_methods[0] || "squash";
}

/** What Merge does, for its tooltip. */
export function mergeTitle(pr: PullRequest): string {
  const m = mergeMethod(pr);
  return `${METHOD_LABEL[m] ?? m}: the project's merge method, set in its Project settings`;
}

const TONE: Record<Tone, string> = {
  good: "text-good",
  warn: "text-warn",
  bad: "text-bad",
  muted: "text-fg-3",
  merged: "text-merged",
};

const ICON: Record<MergeIcon, React.ReactNode> = {
  merged: <GitMerge className="size-4" />,
  closed: <GitPullRequestClosed className="size-4" />,
  draft: <GitPullRequestDraft className="size-4" />,
  conflict: <AlertTriangle className="size-4" />,
  checking: <Loader2 className="size-4 animate-spin" />,
  stacked: <GitPullRequest className="size-4" />,
  blocked: <ShieldAlert className="size-4" />,
  warn: <AlertTriangle className="size-4" />,
  ready: <Check className="size-4" />,
};

export function Checks({ checks }: { checks: PullRequest["checks"] }) {
  if (!checks || !checks.total) return null;
  const { passed, failed, pending, total } = checks;
  return (
    <span className="flex items-center gap-2.5 font-mono text-[11px]">
      {failed > 0 && <span className="flex items-center gap-1 text-bad"><X className="size-3" />{failed} failing</span>}
      {pending > 0 && <span className="flex items-center gap-1 text-warn"><CircleDashed className="size-3" />{pending} pending</span>}
      <span className={cx("flex items-center gap-1", failed || pending ? "text-fg-3" : "text-good")}>
        <Check className="size-3" />{passed}/{total} checks
      </span>
    </span>
  );
}

export function Verdicts({ labels }: { labels: string[] | undefined }) {
  const v = verdicts(labels);
  if (!v.length) return null;
  return (
    <span className="flex flex-wrap items-center gap-1.5" aria-label="Verdicts">
      {v.map((x) => (
        <span key={x.label} className={cx("rounded-full border px-2 py-px font-mono text-[10.5px]", x.tone === "good" ? "border-good/30 bg-good/10 text-good" : "border-bad/30 bg-bad/10 text-bad")}>{x.label}</span>
      ))}
    </span>
  );
}

export function StateBadge({ pr }: { pr: PullRequest }) {
  const [label, cls, icon] =
    pr.state === "merged" ? ["Merged", "bg-merged/15 text-merged border-merged/30", <GitMerge key="i" className="size-3" />]
    : pr.state === "closed" ? ["Closed", "bg-bad/10 text-bad border-bad/25", <GitPullRequestClosed key="i" className="size-3" />]
    : pr.draft ? ["Draft", "bg-white/5 text-fg-2 border-line-strong", <GitPullRequestDraft key="i" className="size-3" />]
    : ["Open", "bg-good/10 text-good border-good/25", <GitPullRequest key="i" className="size-3" />];
  return <span className={cx("inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-[10.5px]", cls)}>{icon}{label}</span>;
}

/** Catch up right away when an open PR's status is stale (e.g. merged on GitHub meanwhile). */
export function useCatchUp(pr: PullRequest) {
  useEffect(() => {
    const stale = !pr.checked_at || Date.now() - new Date(pr.checked_at).getTime() > 15_000;
    if (pr.state === "open" && stale) api.refreshPullRequest(pr.id).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pr.id]);
}

export function PullRequestCard({ pr, after }: { pr: PullRequest; after?: number | null }) {
  const method = mergeMethod(pr);
  const [confirming, setConfirming] = useState(false);
  const refreshing = useAction();
  const merging = useAction();
  const m = mergeability(pr, after);

  useCatchUp(pr);

  useEffect(() => {
    if (pr.state !== "open") setConfirming(false);
  }, [pr.state]);

  // One error line for both: each clears the other's.
  const refresh = () => {
    merging.setError(null);
    return refreshing.run(() => api.refreshPullRequest(pr.id));
  };

  const merge = () => {
    refreshing.setError(null);
    return merging.run(async () => {
      await api.mergePullRequest(pr.id);
      setConfirming(false);
    });
  };
  const err = refreshing.error ?? merging.error;

  const borderTone =
    pr.state === "merged" ? "border-merged/25" : pr.state === "closed" ? "border-line" : m.tone === "bad" ? "border-bad/25" : "border-line-strong";

  return (
    <div className={cx("overflow-hidden rounded-2xl border bg-ink-900/80", borderTone)}>
      {/* header */}
      <div className="px-4 pb-3 pt-3.5">
        <div className="flex items-center gap-2">
          <StateBadge pr={pr} />
          <span className="truncate font-mono text-[11px] text-fg-3">
            {pr.slug} #{pr.number}
          </span>
          <button
            className="ml-auto rounded-full p-1 text-fg-3 transition hover:bg-white/5 hover:text-fg disabled:opacity-40"
            onClick={refresh}
            disabled={refreshing.busy || merging.busy}
            aria-busy={refreshing.busy}
            title={pr.checked_at ? `Checked ${relTime(pr.checked_at)}. Click to refresh` : "Refresh"}
            aria-label="Refresh pull request status"
          >
            <RefreshCw className={cx("size-3.5", refreshing.busy && "animate-spin")} />
          </button>
        </div>
        <a
          href={pr.url}
          target="_blank"
          rel="noopener noreferrer"
          className="group mt-2 flex items-start gap-1.5 text-[15.5px] font-medium leading-snug tracking-tight text-fg hover:text-accent"
        >
          <span>{pr.title || `Pull request #${pr.number}`}</span>
          <ExternalLink className="mt-1 size-3.5 shrink-0 opacity-40 transition group-hover:opacity-100" />
        </a>
        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-fg-3">
          {pr.head_ref && (
            <span className="flex items-center gap-1">
              <span className="rounded bg-white/[0.05] px-1.5 py-px text-fg-2">{pr.head_ref}</span>
              <ArrowRight className="size-3" />
              <span className="rounded bg-white/[0.05] px-1.5 py-px text-fg-2">{pr.base_ref}</span>
            </span>
          )}
          {pr.state === "open" && !!pr.behind_by && <span className="text-warn">{pr.behind_by} behind {pr.base_ref}</span>}
          {(pr.additions > 0 || pr.deletions > 0) && (
            <span>
              <span className="text-good">+{pr.additions}</span> <span className="text-bad">−{pr.deletions}</span>
            </span>
          )}
          {pr.changed_files > 0 && <span>{pr.changed_files} file{pr.changed_files === 1 ? "" : "s"}</span>}
          {pr.commits > 0 && <span>{pr.commits} commit{pr.commits === 1 ? "" : "s"}</span>}
          {pr.comments > 0 && <span>{pr.comments} comment{pr.comments === 1 ? "" : "s"}</span>}
        </div>
      </div>

      {pr.state === "open" && pr.changes_workflows && (
        <Callout tone="warn" icon={<AlertTriangle />} className="mx-4 mb-3">
          It changed <code className="font-mono">.github/workflows/</code> or <code className="font-mono">.github/actions/</code>: on a branch of the repo those run with its secrets as soon as they're pushed, so its CI may already have run. Read the change and its Actions runs.
        </Callout>
      )}

      {/* merge box */}
      <div className="flex flex-wrap items-center gap-3 border-t border-line bg-ink-950/50 px-4 py-3">
        <div className={cx("flex min-w-0 flex-1 basis-48 items-start gap-2", TONE[m.tone])}>
          <span className="mt-px">{ICON[m.icon]}</span>
          <div className="min-w-0">
            <div className="text-[13px]">{m.label}</div>
            {m.detail && <div className="text-[11.5px] text-fg-3">{m.detail}</div>}
            {pr.state === "open" && <div className="mt-1"><Checks checks={pr.checks} /></div>}
            {pr.state === "open" && <div className="mt-1"><Verdicts labels={pr.labels} /></div>}
          </div>
        </div>

        {pr.state === "merged" ? (
          <button className="inline-flex cursor-default items-center gap-1.5 rounded-full border border-merged/30 bg-merged/15 px-3.5 py-1.5 text-[13px] text-merged" disabled>
            <GitMerge className="size-3.5" /> Merged
          </button>
        ) : pr.state === "closed" ? (
          <button className="btn" disabled>
            <GitPullRequestClosed className="size-3.5" /> Closed
          </button>
        ) : confirming ? (
          <div className="flex items-center gap-1.5">
            <button className="btn-ghost" onClick={() => setConfirming(false)} disabled={merging.busy}>
              Cancel
            </button>
            <ActionButton
              variant="bare"
              className="gap-1.5 rounded-full bg-merged px-3.5 py-1.5 text-[13px] font-medium text-ink-950 transition hover:brightness-110"
              onClick={merge}
              pending={merging.busy}
              pendingLabel="Merging…"
              icon={<GitMerge className="size-3.5" />}
            >
              Confirm {method}
            </ActionButton>
          </div>
        ) : (
          <button
            className="inline-flex h-[30px] shrink-0 items-center gap-1.5 rounded-full bg-accent px-3.5 text-[13px] font-medium text-ink-950 transition hover:brightness-110 disabled:bg-ink-700 disabled:text-fg-3 disabled:opacity-50"
            onClick={() => setConfirming(true)}
            disabled={!m.canMerge}
            title={m.canMerge ? mergeTitle(pr) : m.label}
          >
            <GitMerge className="size-3.5" /> Merge
          </button>
        )}
      </div>
      {(err || pr.error) && (
        <Callout tone="bad" icon={<AlertTriangle />} className="mx-4 mb-3">{err ?? pr.error}</Callout>
      )}
    </div>
  );
}
