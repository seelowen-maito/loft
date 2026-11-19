import type { PullRequest } from "./api";
import { relTime } from "./time.ts";

export type Tone = "good" | "warn" | "bad" | "muted" | "merged";
export type MergeIcon = "merged" | "closed" | "draft" | "conflict" | "checking" | "stacked" | "blocked" | "warn" | "ready";

/** Open: GitHub's "open", or a PR Loft hasn't read yet (""). */
export function isOpen(pr: PullRequest) {
  return pr.state === "open" || pr.state === "";
}

/** What the merge box says, in GitHub's own terms; `after` is the open PR this one is stacked on. */
export function mergeability(pr: PullRequest, after?: number | null): { tone: Tone; icon: MergeIcon; label: string; detail?: string; canMerge: boolean } {
  if (pr.state === "merged")
    return {
      tone: "merged",
      icon: "merged",
      label: "Merged",
      detail: [pr.merged_by && `by ${pr.merged_by}`, pr.merged_at && relTime(pr.merged_at)].filter(Boolean).join(" · "),
      canMerge: false,
    };
  if (pr.state === "closed") return { tone: "bad", icon: "closed", label: "Closed without merging", canMerge: false };
  if (pr.draft) return { tone: "muted", icon: "draft", label: "Draft", detail: "Mark it ready for review on GitHub before merging.", canMerge: false };
  if (pr.mergeable === false || pr.mergeable_state === "dirty")
    return { tone: "bad", icon: "conflict", label: "Has merge conflicts", detail: `Conflicts with ${pr.base_ref || "the base branch"} must be resolved first.`, canMerge: false };
  if (pr.mergeable === null || pr.mergeable_state === "unknown" || pr.mergeable_state === "")
    return { tone: "muted", icon: "checking", label: "Checking mergeability…", canMerge: false };
  if (after) return { tone: "muted", icon: "stacked", label: `After #${after}`, detail: `Stacked on #${after}: merge that one first.`, canMerge: false };
  switch (pr.mergeable_state) {
    case "blocked":
      return { tone: "warn", icon: "blocked", label: "Blocked by branch protection", detail: "Required reviews or checks haven't passed yet.", canMerge: true };
    case "behind":
      return { tone: "warn", icon: "warn", label: "Branch is behind base", detail: `No conflicts, but ${pr.head_ref} is behind ${pr.base_ref}.`, canMerge: true };
    case "unstable":
      return { tone: "warn", icon: "warn", label: "No conflicts, but some checks failed", canMerge: true };
    default:
      return { tone: "good", icon: "ready", label: "Ready to merge", detail: "No conflicts with the base branch.", canMerge: true };
  }
}
