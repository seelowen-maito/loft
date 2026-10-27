import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { AlertTriangle, ArrowLeft, CircleCheck, CircleDot, ExternalLink, FolderGit2, KeyRound, MessageSquare, RefreshCw, Search } from "lucide-react";
import { api, type Issue, type IssueDetail, type IssueLink, type ProjectIssues } from "../lib/api";
import { ProjectHeader, useCollapsed } from "../components/ProjectGroup";
import { useServerEvents, type ServerEvent } from "../lib/events";
import { cx, timeAgo } from "../lib/format";

const COLLAPSED_KEY = "loft:collapsed-issue-projects";
const issuePath = (project: string, n: number) => `/issues/${encodeURIComponent(project)}/${n}`;

function StateIcon({ state, className }: { state: string; className?: string }) {
  return state === "open" ? <CircleDot className={cx("text-good", className)} /> : <CircleCheck className={cx("text-[#a371f7]", className)} />;
}

/** A label in its GitHub color, lightened so dark colors still read on the dark background. */
function Label({ name, color, description }: { name: string; color: string; description?: string | null }) {
  const c = /^[0-9a-f]{6}$/i.test(color) ? `#${color}` : "#8b949e";
  return (
    <span
      title={description ?? undefined}
      className="inline-flex items-center rounded-full border px-2 py-px text-[11px] font-medium"
      style={{ backgroundColor: `${c}2e`, borderColor: `${c}66`, color: `color-mix(in srgb, ${c} 55%, white)` }}
    >
      {name}
    </span>
  );
}

function Assignees({ people, size = "size-5" }: { people: Issue["assignees"]; size?: string }) {
  if (!people.length) return null;
  return (
    <span className="flex -space-x-1.5">
      {people.map((p) => <img key={p.login} src={p.avatar_url} alt={p.login} title={p.login} className={cx("rounded-full border border-ink-950 bg-ink-800", size)} />)}
    </span>
  );
}

export default function IssuesPage() {
  const { project: routeProject, number: routeNumber } = useParams();
  const [data, setData] = useState<{ connected: boolean; projects: ProjectIssues[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [state, setState] = useState<"open" | "closed">("open");
  // Bumped when Loft publishes on a project, so the open issue refetches too.
  const [published, setPublished] = useState(0);
  // Project groups folded away in the list, remembered per browser; new projects start expanded.
  const folded = useCollapsed(COLLAPSED_KEY);

  const load = useCallback(() => {
    setLoading(true);
    api.issues().then((d) => (setData(d), setError(null))).catch((e) => setError(String(e.message ?? e))).finally(() => setLoading(false));
  }, []);
  useEffect(load, [load]);
  // A publish touches one project: refetch just its list, or everything if the tab doesn't have it yet.
  const loaded = useRef(data);
  loaded.current = data;
  const reload = useCallback((project: string) => {
    if (!loaded.current?.projects.some((p) => p.project === project)) return load();
    api.issues(project).then((d) => setData((cur) => cur && { ...cur, projects: cur.projects.map((p) => (p.project === project ? (d.projects[0] ?? p) : p)) })).catch(load);
  }, [load]);
  useServerEvents(
    useCallback((e: ServerEvent) => {
      if (e.type === "issues") {
        reload(e.project);
        if (e.project === routeProject) setPublished((n) => n + 1);
      }
    }, [reload, routeProject]),
  );

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (data?.projects ?? []).map((p) => ({
      ...p,
      shown: p.issues.filter((i) => i.state === state && (!q || i.title.toLowerCase().includes(q) || `#${i.number}` === q || i.labels.some((l) => l.name.toLowerCase().includes(q)))),
    }));
  }, [data, query, state]);
  const count = (s: string) => (data?.projects ?? []).reduce((n, p) => n + p.issues.filter((i) => i.state === s).length, 0);
  const selected = routeProject && routeNumber ? { project: routeProject, number: Number(routeNumber) } : null;

  return (
    <div className="grid h-full grid-cols-[420px_minmax(0,1fr)] overflow-hidden max-lg:grid-cols-1">
      <aside className={cx("flex min-h-0 flex-col border-r border-line bg-ink-950", selected && "max-lg:hidden")}>
        <div className="space-y-3 px-4 pb-3 pt-5">
          <div className="flex items-center justify-between gap-3">
            <div>
              <div className="eyebrow">Issues</div>
              <div className="mt-1 font-mono text-[11px] text-fg-3">
                {data ? `${count("open")} open · ${count("closed")} closed · from GitHub` : "loading…"}
              </div>
            </div>
            <button className="btn-ghost size-8 justify-center p-0" onClick={load} aria-label="Refresh" title="Refresh">
              <RefreshCw className={cx("size-4", loading && "animate-spin")} />
            </button>
          </div>
          <label className="flex items-center gap-2 rounded-xl border border-line-strong bg-ink-850 px-3 py-1.5">
            <Search className="size-3.5 text-fg-3" />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search issues" className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3" />
          </label>
          <div className="flex gap-1">
            {(["open", "closed"] as const).map((s) => (
              <button key={s} onClick={() => setState(s)} className={cx("chip capitalize", state === s ? "border-line-strong bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
                <StateIcon state={s} className="size-3" /> {s} {data && <span className="text-fg-3">{count(s)}</span>}
              </button>
            ))}
          </div>
        </div>
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-2 pb-6">
          {error && <Notice icon={AlertTriangle} title="Couldn't load issues.">{error}</Notice>}
          {data && !data.connected && (
            <Notice icon={KeyRound} title="GitHub isn't connected.">
              Add a GitHub token in <Link to="/settings" className="text-accent underline decoration-accent/30 underline-offset-2">Settings</Link> or run <code className="font-mono">gh auth login</code> on this Mac to see each project's issues.
            </Notice>
          )}
          {data && !data.projects.length && (
            <Notice icon={FolderGit2} title="No projects yet.">Issues are shown per project: set a project on an agent in its Config tab.</Notice>
          )}
          {visible.map((p) => {
            const open = folded.isOpen(p.project);
            return (
              <section key={p.project}>
                <div className="flex items-center gap-2 pb-1 pr-2.5">
                  <ProjectHeader name={p.project} open={open} onToggle={() => folded.toggle(p.project)} className="min-w-0">
                    {!p.problem && !p.error && <span title={`${p.shown.length} ${state} issues`}>{p.shown.length}</span>}
                  </ProjectHeader>
                  {p.repo && (
                    <a href={`https://github.com/${p.repo}/issues`} target="_blank" rel="noreferrer" className="flex shrink-0 items-center gap-1 pt-3 font-mono text-[10.5px] text-fg-3 hover:text-fg">
                      {p.repo} <ExternalLink className="size-3" />
                    </a>
                  )}
                </div>
                {!open ? null : p.problem ? (
                  <div className="mx-2.5 rounded-xl border border-line px-3 py-2 text-[12.5px] text-fg-3">No GitHub remote: {p.problem}</div>
                ) : p.error ? (
                  <div className="mx-2.5 rounded-xl border border-bad/20 bg-bad/[0.05] px-3 py-2 text-[12.5px] text-fg-2">{p.error}</div>
                ) : data?.connected && !p.shown.length ? (
                  <div className="px-2.5 py-1 text-[12.5px] text-fg-3">No {state} issues{query.trim() ? " match" : ""}.</div>
                ) : (
                  p.shown.map((i) => <IssueRow key={i.number} project={p.project} i={i} active={selected?.project === p.project && selected.number === i.number} />)
                )}
                {open && p.truncated && (
                  <div className="px-2.5 py-1 text-[12px] text-fg-3">
                    Showing the first 1,000 issues and pull requests, open ones first. The rest are on <a href={`https://github.com/${p.repo}/issues`} target="_blank" rel="noreferrer" className="text-accent underline decoration-accent/30 underline-offset-2">GitHub</a>.
                  </div>
                )}
              </section>
            );
          })}
        </div>
      </aside>
      <section className={cx("min-h-0 overflow-y-auto", !selected && "max-lg:hidden")}>
        {selected ? (
          <IssueView key={`${selected.project}#${selected.number}`} project={selected.project} number={selected.number} published={published} />
        ) : (
          <div className="flex h-full items-center justify-center text-[13px] text-fg-3">Pick an issue to read it.</div>
        )}
      </section>
    </div>
  );
}

function Notice({ icon: Icon, title, children }: { icon: typeof KeyRound; title: string; children: React.ReactNode }) {
  return (
    <div className="mx-2 flex items-start gap-3 rounded-2xl border border-warn/20 bg-warn/[0.06] px-4 py-3 text-[13px]">
      <Icon className="mt-0.5 size-4 shrink-0 text-warn" />
      <div className="text-fg-2">
        <span className="text-fg">{title}</span> {children}
      </div>
    </div>
  );
}

function IssueRow({ project, i, active }: { project: string; i: Issue; active: boolean }) {
  return (
    <Link to={issuePath(project, i.number)} className={cx("flex gap-2.5 rounded-xl px-2.5 py-2.5 transition", active ? "bg-white/[0.06]" : "hover:bg-white/[0.03]")}>
      <StateIcon state={i.state} className="mt-0.5 size-4 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="text-[13.5px] leading-snug text-fg">
          {i.title} {i.labels.map((l) => <span key={l.name} className="ml-1 align-middle"><Label {...l} /></span>)}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[10.5px] text-fg-3">
          <span>#{i.number} · {i.state === "open" ? `opened ${timeAgo(i.created_at)}` : `updated ${timeAgo(i.updated_at)}`}</span>
          {i.sub_issues.total > 0 && <span>{i.sub_issues.completed}/{i.sub_issues.total} sub-issues</span>}
          {i.blocked_by > 0 && <span className="text-warn">blocked by {i.blocked_by}</span>}
          {i.blocking > 0 && <span>blocking {i.blocking}</span>}
          {i.comments > 0 && <span className="flex items-center gap-0.5"><MessageSquare className="size-3" />{i.comments}</span>}
        </div>
      </div>
      <Assignees people={i.assignees} />
    </Link>
  );
}

function IssueView({ project, number, published }: { project: string; number: number; published: number }) {
  const [issue, setIssue] = useState<IssueDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.issue(project, number).then((i) => (setIssue(i), setError(null))).catch((e) => setError(String(e.message ?? e)));
  }, [project, number, published]);

  if (error) return <div className="p-8 text-[13px] text-bad">{error}</div>;
  if (!issue) return <div className="mx-auto max-w-3xl space-y-3 p-8"><div className="skeleton h-7 w-2/3 rounded-lg" /><div className="skeleton h-40 rounded-xl" /></div>;
  return (
    <article className="mx-auto max-w-3xl space-y-6 px-6 py-8 max-md:px-4">
      <Link to="/issues" className="btn-ghost -ml-2.5 lg:hidden"><ArrowLeft className="size-4" /> Issues</Link>
      <header className="space-y-3">
        <div className="font-mono text-[11px] text-fg-3">{issue.repo}</div>
        <h1 className="text-[22px] font-medium leading-tight tracking-tight">
          {issue.title} <span className="font-normal text-fg-3">#{issue.number}</span>
        </h1>
        <div className="flex flex-wrap items-center gap-3 text-[12.5px] text-fg-3">
          <span className={cx("inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[12px] font-medium text-ink-950", issue.state === "open" ? "bg-good" : "bg-[#a371f7]")}>
            <StateIcon state={issue.state} className="size-3.5 !text-ink-950" /> {issue.state === "open" ? "Open" : issue.state_reason === "not_planned" ? "Closed as not planned" : "Closed"}
          </span>
          <span>{issue.author} opened {timeAgo(issue.created_at)}</span>
          <a href={issue.url} target="_blank" rel="noreferrer" className="btn ml-auto"><ExternalLink className="size-3.5" /> Open on GitHub</a>
        </div>
      </header>
      {(issue.labels.length > 0 || issue.assignees.length > 0) && (
        <div className="flex flex-wrap gap-x-8 gap-y-3">
          {issue.labels.length > 0 && (
            <div>
              <div className="eyebrow mb-1.5">Labels</div>
              <div className="flex flex-wrap gap-1">{issue.labels.map((l) => <Label key={l.name} {...l} />)}</div>
            </div>
          )}
          {issue.assignees.length > 0 && (
            <div>
              <div className="eyebrow mb-1.5">Assignees</div>
              <div className="flex flex-wrap gap-3">
                {issue.assignees.map((a) => (
                  <span key={a.login} className="flex items-center gap-1.5 text-[13px] text-fg-2">
                    <img src={a.avatar_url} alt="" className="size-5 rounded-full bg-ink-800" /> {a.login}
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
      <div className="panel px-5 py-4">
        {issue.body.trim() ? (
          <div className="prose-chat text-[14px] text-fg-2">
            <Markdown remarkPlugins={[remarkGfm]}>{issue.body}</Markdown>
          </div>
        ) : (
          <div className="text-[13px] italic text-fg-3">No description provided.</div>
        )}
      </div>
      <Links title={`Sub-issues · ${issue.sub_issue_list.filter((i) => i.state === "closed").length}/${issue.sub_issue_list.length} done`} items={issue.sub_issue_list} issue={issue} />
      <Links title="Blocked by" items={issue.blocked_by_list} issue={issue} />
      <Links title="Blocking" items={issue.blocking_list} issue={issue} />
    </article>
  );
}

/** Linked issues; those in the same repo open here, others on GitHub. */
function Links({ title, items, issue }: { title: string; items: IssueLink[]; issue: IssueDetail }) {
  if (!items.length) return null;
  const local = (l: IssueLink) => l.url === `https://github.com/${issue.repo}/issues/${l.number}`;
  return (
    <section>
      <div className="eyebrow mb-2">{title}</div>
      <ul className="divide-y divide-line rounded-2xl border border-line">
        {items.map((l) => {
          const inner = (
            <>
              <StateIcon state={l.state} className="size-4 shrink-0" />
              <span className="min-w-0 flex-1 truncate text-fg">{l.title}</span>
              <span className="font-mono text-[11px] text-fg-3">#{l.number}</span>
              {!local(l) && <ExternalLink className="size-3.5 text-fg-3" />}
            </>
          );
          const cls = "flex items-center gap-2.5 px-3.5 py-2.5 text-[13px] transition hover:bg-white/[0.03]";
          return (
            <li key={l.url}>
              {local(l) ? <Link to={issuePath(issue.project, l.number)} className={cls}>{inner}</Link> : <a href={l.url} target="_blank" rel="noreferrer" className={cls}>{inner}</a>}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
