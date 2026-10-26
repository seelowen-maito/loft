import { useEffect, useState } from "react";
import {
  AlertTriangle, ArrowRight, Check, CircleDashed, ExternalLink, GitMerge, GitPullRequest, GitPullRequestClosed,
  GitPullRequestDraft, Loader2, RefreshCw, ShieldAlert, X,
} from "lucide-react";
import { api, type PullRequest } from "../../lib/api";
import { cx, timeAgo } from "../../lib/format";

const METHOD_LABEL: Record<string, string> = { squash: "Squash and merge", merge: "Create merge commit", rebase: "Rebase and merge" };

type Tone = "good" | "warn" | "bad" | "muted" | "merged";
const TONE: Record<Tone, string> = {
  good: "text-good",
  warn: "text-warn",
  bad: "text-bad",
  muted: "text-fg-3",
  merged: "text-merged",
};

/** What the merge box says, in GitHub's own terms. */
function mergeability(pr: PullRequest): { tone: Tone; icon: React.ReactNode; label: string; detail?: string; canMerge: boolean } {
  if (pr.state === "merged")
    return {
      tone: "merged",
      icon: <GitMerge className="size-4" />,
      label: "Merged",
      detail: [pr.merged_by && `by ${pr.merged_by}`, pr.merged_at && timeAgo(pr.merged_at)].filter(Boolean).join(" · "),
      canMerge: false,
    };
  if (pr.state === "closed")
    return { tone: "bad", icon: <GitPullRequestClosed className="size-4" />, label: "Closed without merging", canMerge: false };
  if (pr.draft)
    return { tone: "muted", icon: <GitPullRequestDraft className="size-4" />, label: "Draft", detail: "Mark it ready for review on GitHub before merging.", canMerge: false };
  if (pr.mergeable === false || pr.mergeable_state === "dirty")
    return { tone: "bad", icon: <AlertTriangle className="size-4" />, label: "Has merge conflicts", detail: `Conflicts with ${pr.base_ref || "the base branch"} must be resolved first.`, canMerge: false };
  if (pr.mergeable === null || pr.mergeable_state === "unknown" || pr.mergeable_state === "")
    return { tone: "muted", icon: <Loader2 className="size-4 animate-spin" />, label: "Checking mergeability…", canMerge: false };
  switch (pr.mergeable_state) {
    case "blocked":
      return { tone: "warn", icon: <ShieldAlert className="size-4" />, label: "Blocked by branch protection", detail: "Required reviews or checks haven't passed yet.", canMerge: true };
    case "behind":
      return { tone: "warn", icon: <AlertTriangle className="size-4" />, label: "Branch is behind base", detail: `No conflicts, but ${pr.head_ref} is behind ${pr.base_ref}.`, canMerge: true };
    case "unstable":
      return { tone: "warn", icon: <AlertTriangle className="size-4" />, label: "No conflicts, but some checks failed", canMerge: true };
    default:
      return { tone: "good", icon: <Check className="size-4" />, label: "Ready to merge", detail: "No conflicts with the base branch.", canMerge: true };
  }
}

function Checks({ checks }: { checks: PullRequest["checks"] }) {
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

function StateBadge({ pr }: { pr: PullRequest }) {
  const [label, cls, icon] =
    pr.state === "merged" ? ["Merged", "bg-merged/15 text-merged border-merged/30", <GitMerge key="i" className="size-3" />]
    : pr.state === "closed" ? ["Closed", "bg-bad/10 text-bad border-bad/25", <GitPullRequestClosed key="i" className="size-3" />]
    : pr.draft ? ["Draft", "bg-white/5 text-fg-2 border-line-strong", <GitPullRequestDraft key="i" className="size-3" />]
    : ["Open", "bg-good/10 text-good border-good/25", <GitPullRequest key="i" className="size-3" />];
  return <span className={cx("inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-[10.5px]", cls)}>{icon}{label}</span>;
}

export function PullRequestCard({ pr }: { pr: PullRequest }) {
  const methods = pr.merge_methods.length ? pr.merge_methods : ["squash", "merge", "rebase"];
  const [method, setMethod] = useState(methods[0]);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState<"merge" | "refresh" | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const m = mergeability(pr);

  // Catch up right away when an open PR's status is stale (e.g. merged on GitHub meanwhile).
  useEffect(() => {
    const stale = !pr.checked_at || Date.now() - new Date(pr.checked_at).getTime() > 15_000;
    if (pr.state === "open" && stale) api.refreshPullRequest(pr.id).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pr.id]);

  useEffect(() => {
    if (pr.state !== "open") setConfirming(false);
  }, [pr.state]);

  const refresh = async () => {
    setBusy("refresh");
    setErr(null);
    try {
      await api.refreshPullRequest(pr.id);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(null);
    }
  };

  const merge = async () => {
    setBusy("merge");
    setErr(null);
    try {
      await api.mergePullRequest(pr.id, method);
      setConfirming(false);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(null);
    }
  };

  const borderTone =
    pr.state === "merged" ? "border-merged/25" : pr.state === "closed" ? "border-line" : m.tone === "bad" ? "border-bad/25" : "border-line-strong";

  return (
    <div className="pl-9">
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
              disabled={busy !== null}
              title={pr.checked_at ? `Checked ${timeAgo(pr.checked_at)}. Click to refresh` : "Refresh"}
              aria-label="Refresh pull request status"
            >
              <RefreshCw className={cx("size-3.5", busy === "refresh" && "animate-spin")} />
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

        {/* merge box */}
        <div className="flex flex-wrap items-center gap-3 border-t border-line bg-ink-950/50 px-4 py-3">
          <div className={cx("flex min-w-0 flex-1 items-start gap-2", TONE[m.tone])}>
            <span className="mt-px">{m.icon}</span>
            <div className="min-w-0">
              <div className="text-[13px]">{m.label}</div>
              {m.detail && <div className="text-[11.5px] text-fg-3">{m.detail}</div>}
              {pr.state === "open" && <div className="mt-1"><Checks checks={pr.checks} /></div>}
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
              <button className="btn-ghost" onClick={() => setConfirming(false)} disabled={busy === "merge"}>
                Cancel
              </button>
              <button
                className="inline-flex items-center gap-1.5 rounded-full bg-merged px-3.5 py-1.5 text-[13px] font-medium text-ink-950 transition hover:brightness-110 disabled:opacity-50"
                onClick={merge}
                disabled={busy === "merge"}
              >
                {busy === "merge" ? <Loader2 className="size-3.5 animate-spin" /> : <GitMerge className="size-3.5" />}
                Confirm {method}
              </button>
            </div>
          ) : (
            <div className="flex items-center">
              {methods.length > 1 && (
                <select
                  className="h-[30px] rounded-l-full border border-r-0 border-line-strong bg-ink-800 pl-3 pr-1 text-[12px] text-fg-2 outline-none disabled:opacity-40"
                  value={method}
                  onChange={(e) => setMethod(e.target.value)}
                  disabled={!m.canMerge}
                  aria-label="Merge method"
                >
                  {methods.map((mm) => (
                    <option key={mm} value={mm}>{METHOD_LABEL[mm] ?? mm}</option>
                  ))}
                </select>
              )}
              <button
                className={cx(
                  "inline-flex h-[30px] items-center gap-1.5 bg-accent px-3.5 text-[13px] font-medium text-ink-950 transition hover:brightness-110 disabled:bg-ink-700 disabled:text-fg-3",
                  methods.length > 1 ? "rounded-r-full" : "rounded-full",
                )}
                onClick={() => setConfirming(true)}
                disabled={!m.canMerge}
                title={m.canMerge ? undefined : m.label}
              >
                <GitMerge className="size-3.5" /> Merge
              </button>
            </div>
          )}
        </div>
        {(err || pr.error) && (
          <div className="flex items-center gap-1.5 border-t border-bad/20 bg-bad/[0.06] px-4 py-2 text-[12px] text-bad">
            <AlertTriangle className="size-3.5 shrink-0" /> {err ?? pr.error}
          </div>
        )}
      </div>
    </div>
  );
}
