import type { WaitsOn } from "./api";

/** The specs to start first: each other spec that owns an open blocker and hasn't started, once. */
export function specsToStartFirst(waits: WaitsOn[] | undefined): { repo: string; spec: number; issueId: string }[] {
  const out: { repo: string; spec: number; issueId: string }[] = [];
  for (const { delivery: d } of waits ?? [])
    if (d && !d.started && d.issue_id && !out.some((o) => o.repo === d.repo && o.spec === d.spec)) out.push({ repo: d.repo, spec: d.spec, issueId: d.issue_id });
  return out;
}
