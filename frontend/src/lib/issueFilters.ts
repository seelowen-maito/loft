/** The Issues list's filters, kept in the URL so back/forward, reload and shared links show the same list. */
export type Source = "all" | "loft" | "github";

export type IssueFilters = {
  /** Search text: words, a label, `@owner`, or `#42`. */
  q: string;
  source: Source;
  project: string;
  /** An agent id. */
  agent: string;
  /** Only issues you created. */
  mine: boolean;
  /** The Done group unfolded. */
  closed: boolean;
};

export const DEFAULT_FILTERS: IssueFilters = { q: "", source: "all", project: "", agent: "", mine: false, closed: false };

const SOURCES: Source[] = ["all", "loft", "github"];
const MAX = 200;

/** Filters from a query string. Anything unknown, invalid or too long falls back to the default. */
export function parseFilters(params: URLSearchParams): IssueFilters {
  const text = (k: string) => {
    const v = (params.get(k) ?? "").trim();
    return v.length <= MAX ? v : "";
  };
  const flag = (k: string) => params.get(k) === "1";
  const source = params.get("source") as Source;
  return {
    q: text("q"),
    source: SOURCES.includes(source) ? source : "all",
    project: text("project"),
    agent: text("agent"),
    mine: flag("mine"),
    closed: flag("closed"),
  };
}

/** The query string for `f`, without the filters at their default, so a plain list is a plain `/tasks`. */
export function filtersToParams(f: IssueFilters): URLSearchParams {
  const p = new URLSearchParams();
  if (f.q.trim()) p.set("q", f.q.trim());
  if (f.source !== "all") p.set("source", f.source);
  if (f.project) p.set("project", f.project);
  if (f.agent) p.set("agent", f.agent);
  if (f.mine) p.set("mine", "1");
  if (f.closed) p.set("closed", "1");
  return p;
}
