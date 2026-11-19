import type { PullRequest } from "./api";

/** A pull request a message names: by `#N`, `owner/repo#N` or its GitHub link. */
type Ref = { slug: string | null; number: number };

// `owner/repo#N` or `#N` starting a word, or a GitHub pull request link. Like the backend's tags (tags.rs).
const REF = /(?<![\w/#])(?:([\w.-]+\/[\w.-]+))?#(\d{1,9})(?![\w/]|\.\w)|https?:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/pull\/(\d{1,9})/g;

/** The pull requests `text` names, in order and without repeats. Code isn't searched. */
export function parseRefs(text: string): Ref[] {
  const prose = text.replace(/```[\s\S]*?```|`[^`\n]*`/g, " ");
  const out: Ref[] = [];
  for (const m of prose.matchAll(REF)) {
    const slug = (m[1] ?? m[3])?.toLowerCase() ?? null;
    const number = Number(m[2] ?? m[4]);
    if (!out.some((r) => r.slug === slug && r.number === number)) out.push({ slug, number });
  }
  return out;
}

/**
 * The tracked pull requests `text` names, in the order it names them. A bare `#N` means the agent's `project`
 * (or the only tracked PR with that number when it has none). A PR tracked for several agents counts once,
 * as its most recently checked copy.
 */
export function namedPrs(text: string, project: string, prs: PullRequest[]): PullRequest[] {
  const repo = (p: PullRequest) => p.slug.split("/").pop() ?? "";
  const out: PullRequest[] = [];
  for (const r of parseRefs(text)) {
    let hits = prs.filter((p) => p.number === r.number && (r.slug ? p.slug.toLowerCase() === r.slug : !project || repo(p) === project));
    if (!r.slug && !project && new Set(hits.map((p) => p.slug)).size > 1) hits = [];
    const pr = hits.sort((a, b) => (b.checked_at ?? "").localeCompare(a.checked_at ?? ""))[0];
    if (pr && !out.some((p) => p.slug === pr.slug && p.number === pr.number)) out.push(pr);
  }
  return out;
}

/** A row of the merge-order card: the PR and, when it's stacked on another open PR of the list, that PR's number. */
export type MergeRow = { pr: PullRequest; after: number | null };

/**
 * The merge order of `prs` (as `namedPrs` returns them): open ones as named, then merged and closed ones.
 * A PR whose base branch is another listed open PR's head is stacked on it and waits for it.
 */
export function mergeOrder(prs: PullRequest[]): MergeRow[] {
  const open = (p: PullRequest) => p.state === "open" || p.state === "";
  const rows = prs.map((pr) => {
    const base = open(pr) ? prs.find((o) => o !== pr && open(o) && o.slug === pr.slug && !!o.head_ref && o.head_ref === pr.base_ref) : undefined;
    return { pr, after: base?.number ?? null };
  });
  return [...rows.filter((r) => open(r.pr)), ...rows.filter((r) => !open(r.pr))];
}
