import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";
import {
  AlertTriangle, ArrowDown, ArrowUp, CircleDot, ExternalLink, FolderGit2, GitCommitHorizontal,
  GitPullRequest, Globe, KeyRound, Lock, RefreshCw, Search, Star, X,
} from "lucide-react";
import { api, type Commit, type GithubInfo, type GithubItem, type Info, type Repo, type Site } from "../lib/api";
import { cx, fmtNum, timeAgo } from "../lib/format";
import { Heatmap, ShareBars, WeeklyBars } from "../components/Charts";
import { AgentActivity } from "./AgentActivity";
import { pickGreeting } from "../lib/greetings";

type Sort = "activity" | "name" | "commits";

function sum(a: number[]) {
  return a.reduce((x, y) => x + y, 0);
}

function weekly(daily: number[]) {
  const out: number[] = [];
  for (let end = daily.length; end > 0; end -= 7) out.push(sum(daily.slice(Math.max(0, end - 7), end)));
  return out;
}

function lastActivity(r: Repo) {
  return r.last_commit?.timestamp ?? 0;
}

export default function ReposPage() {
  const [repos, setRepos] = useState<Repo[] | null>(null);
  const [gh, setGh] = useState<Record<string, GithubInfo>>({});
  const [ghLoading, setGhLoading] = useState(true);
  const [info, setInfo] = useState<Info | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<Sort>("activity");
  const [loadedAt, setLoadedAt] = useState<Date | null>(null);
  const [greeting] = useState(pickGreeting);

  const load = async (refresh = false) => {
    setRefreshing(true);
    try {
      const r = await api.repos(refresh);
      setRepos(r);
      setLoadedAt(new Date());
    } finally {
      setRefreshing(false);
    }
    setGhLoading(true);
    api.github(refresh).then(setGh).finally(() => setGhLoading(false));
  };

  useEffect(() => {
    load();
    api.info().then(setInfo).catch(() => {});
  }, []);

  const totals = useMemo(() => {
    const rs = repos ?? [];
    const daily = Array(182).fill(0) as number[];
    rs.forEach((r) => r.daily.forEach((v, i) => (daily[i] += v)));
    const ghs = Object.values(gh);
    return {
      daily,
      commits30: sum(daily.slice(-30)),
      commitsPrev30: sum(daily.slice(-60, -30)),
      prs: ghs.reduce((a, g) => a + g.pulls.length, 0),
      issues: ghs.reduce((a, g) => a + g.issues.length, 0),
      dirty: rs.filter((r) => r.staged + r.modified + r.untracked > 0).length,
      ghErrors: ghs.filter((g) => g.error).length,
    };
  }, [repos, gh]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rs = (repos ?? []).filter((r) => !q || r.name.toLowerCase().includes(q) || r.languages.some(([l]) => l.toLowerCase().includes(q)));
    return [...rs].sort((a, b) =>
      sort === "name" ? a.name.localeCompare(b.name) : sort === "commits" ? b.total_commits - a.total_commits : lastActivity(b) - lastActivity(a),
    );
  }, [repos, query, sort]);

  const allPulls = useMemo(
    () => Object.entries(gh).flatMap(([repo, g]) => g.pulls.map((p) => ({ repo, ...p }))).sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
    [gh],
  );
  const allIssues = useMemo(
    () => Object.entries(gh).flatMap(([repo, g]) => g.issues.map((p) => ({ repo, ...p }))).sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
    [gh],
  );

  const mostActive = useMemo(
    () =>
      (repos ?? [])
        .map((r) => ({ name: r.name, n: sum(r.daily.slice(-30)) }))
        .filter((r) => r.n > 0)
        .sort((a, b) => b.n - a.n)
        .slice(0, 5),
    [repos],
  );

  const delta = totals.commits30 - totals.commitsPrev30;
  const sel = repos?.find((r) => r.name === selected) ?? null;

  return (
    <div className="h-full overflow-y-auto">
      {/* hero */}
      <section className="relative overflow-hidden border-b border-line">
        <div className="hairline-grid absolute inset-0 [mask-image:linear-gradient(to_bottom,black,transparent)]" />
        <div className="pointer-events-none absolute -top-40 right-[-10%] h-[420px] w-[620px] rounded-full bg-[radial-gradient(closest-side,rgb(200_242_109/0.13),transparent)] blur-2xl" />
        <div className="pointer-events-none absolute -top-20 right-[25%] h-[300px] w-[400px] rounded-full bg-[radial-gradient(closest-side,rgb(140_184_255/0.07),transparent)] blur-2xl" />
        <div className="relative mx-auto max-w-[1600px] px-5 pb-10 pt-12">
          <div className="flex flex-wrap items-end justify-between gap-6">
            <div className="animate-rise">
              <div className="eyebrow mb-4">{info?.projects_dir ?? "~/projects"} · {repos?.length ?? "—"} repositories</div>
              <h1 className="max-w-3xl text-balance font-serif text-[52px] leading-[1.02] tracking-tight text-fg">
                {/* each half stays whole, so lines only break between them */}
                <span className="inline-block">{greeting[0]}</span> <em className="inline-block text-accent">{greeting[1]}</em>
              </h1>
            </div>
            <div className="flex items-center gap-3">
              {loadedAt && <span className="font-mono text-[11px] text-fg-3">updated {timeAgo(loadedAt.toISOString())}</span>}
              <button className="btn" onClick={() => load(true)} disabled={refreshing}>
                <RefreshCw className={cx("size-3.5", refreshing && "animate-spin")} /> Refresh
              </button>
            </div>
          </div>

          <div className="mt-10 grid grid-cols-2 gap-px overflow-hidden rounded-2xl border border-line bg-line md:grid-cols-5">
            <Stat label="Repositories" value={repos ? String(repos.length) : null} />
            <Stat
              label="Commits · 30d"
              value={repos ? fmtNum(totals.commits30) : null}
              sub={repos ? `${delta >= 0 ? "+" : ""}${delta} vs prior 30d` : undefined}
            />
            <Stat label="Open pull requests" value={ghLoading && !allPulls.length ? null : String(totals.prs)} />
            <Stat label="Open issues" value={ghLoading && !allIssues.length ? null : String(totals.issues)} />
            <Stat label="Uncommitted work" value={repos ? String(totals.dirty) : null} sub="repos with local changes" />
          </div>
        </div>
      </section>

      <div className="mx-auto max-w-[1600px] space-y-6 px-5 py-8">
        {info && !info.github_token && totals.ghErrors > 0 && (
          <div className="flex items-start gap-3 rounded-2xl border border-warn/20 bg-warn/[0.06] px-4 py-3 text-[13px]">
            <KeyRound className="mt-0.5 size-4 shrink-0 text-warn" />
            <div className="text-fg-2">
              <span className="text-fg">GitHub data is limited.</span> {totals.ghErrors} repo{totals.ghErrors > 1 ? "s" : ""} couldn't be read,
              probably because they're private. Add a GitHub token in <Link to="/settings" className="text-accent underline decoration-accent/30 underline-offset-2">Settings</Link> to see
              their pull requests and issues.
            </div>
          </div>
        )}

        <AgentActivity />

        {/* activity */}
        <section className="grid gap-6 lg:grid-cols-[1fr_320px]">
          <div className="panel p-5">
            <div className="mb-4 flex items-baseline justify-between">
              <div>
                <div className="eyebrow">Activity</div>
                <div className="mt-1 text-[15px]">Commits across all repositories · last 26 weeks</div>
              </div>
              <div className="font-mono text-[12px] text-fg-3">{fmtNum(sum(totals.daily))} total</div>
            </div>
            {repos ? (
              <div className="flex flex-wrap items-start gap-8">
                <div className="overflow-x-auto pb-1">
                  <Heatmap daily={totals.daily} cell={12} gap={3} />
                </div>
                <div className="min-w-[240px] flex-1">
                  <div className="mb-2 flex items-baseline justify-between font-mono text-[10.5px] text-fg-3">
                    <span>Weekly commits</span>
                    <span>peak {Math.max(0, ...weekly(totals.daily))}/wk</span>
                  </div>
                  <WeeklyBars daily={totals.daily} height={118} />
                </div>
              </div>
            ) : (
              <div className="skeleton h-[124px] rounded-lg" />
            )}
          </div>
          <div className="panel p-5">
            <div className="eyebrow mb-3">Most active · 30d</div>
            {mostActive.length === 0 && <div className="text-[13px] text-fg-3">No commits in the last 30 days.</div>}
            <ol className="space-y-2.5">
              {mostActive.map((r, i) => (
                <li key={r.name}>
                  <button className="group flex w-full items-center gap-3 text-left" onClick={() => setSelected(r.name)}>
                    <span className="w-4 font-mono text-[11px] text-fg-3">{i + 1}</span>
                    <span className="flex-1 truncate text-[13.5px] group-hover:text-accent">{r.name}</span>
                    <span className="h-1 w-16 overflow-hidden rounded-full bg-ink-700">
                      <span className="block h-full rounded-full bg-accent" style={{ width: `${(r.n / mostActive[0].n) * 100}%` }} />
                    </span>
                    <span className="w-8 text-right font-mono text-[11px] text-fg-2">{r.n}</span>
                  </button>
                </li>
              ))}
            </ol>
          </div>
        </section>

        {/* PRs + issues */}
        <section className="grid gap-6 lg:grid-cols-2">
          <ItemList
            title="Open pull requests"
            icon={<GitPullRequest className="size-3.5" />}
            items={allPulls}
            loading={ghLoading && !allPulls.length}
            empty="No open pull requests."
            onRepo={setSelected}
          />
          <ItemList
            title="Open issues"
            icon={<CircleDot className="size-3.5" />}
            items={allIssues}
            loading={ghLoading && !allIssues.length}
            empty="No open issues."
            onRepo={setSelected}
            seeAll="/tasks?source=github"
          />
        </section>

        {/* repo grid */}
        <section>
          <div className="mb-4 flex flex-wrap items-center gap-3">
            <div className="eyebrow mr-auto">Repositories</div>
            <label className="relative">
              <Search className="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-fg-3" />
              <input className="input w-56 rounded-full py-1.5 pl-8" placeholder="Filter by name or language" value={query} onChange={(e) => setQuery(e.target.value)} />
            </label>
            <div className="flex rounded-full border border-line bg-ink-900 p-0.5">
              {(["activity", "name", "commits"] as Sort[]).map((s) => (
                <button
                  key={s}
                  onClick={() => setSort(s)}
                  className={cx("rounded-full px-3 py-1 text-[12px] capitalize", sort === s ? "bg-white/10 text-fg" : "text-fg-3 hover:text-fg")}
                >
                  {s}
                </button>
              ))}
            </div>
          </div>
          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
            {!repos && Array.from({ length: 6 }).map((_, i) => <div key={i} className="skeleton h-[230px] rounded-2xl" />)}
            {visible.map((r, i) => (
              <RepoCard key={r.name} repo={r} gh={gh[r.name]} index={i} onOpen={() => setSelected(r.name)} />
            ))}
          </div>
          {repos && repos.length === 0 && (
            <div className="panel p-10 text-center text-fg-2">No git repositories found in {info?.projects_dir}.</div>
          )}
        </section>
      </div>

      {sel && <RepoDrawer repo={sel} gh={gh[sel.name]} onClose={() => setSelected(null)} />}
    </div>
  );
}

function Stat({ label, value, sub }: { label: string; value: string | null; sub?: string }) {
  return (
    <div className="bg-ink-900 px-5 py-4">
      <div className="eyebrow">{label}</div>
      {value === null ? (
        <div className="skeleton mt-2 h-9 w-16 rounded-md" />
      ) : (
        <div className="mt-1.5 font-serif text-[40px] leading-none tracking-tight tabular-nums">{value}</div>
      )}
      {sub && <div className="mt-1.5 font-mono text-[11px] text-fg-3">{sub}</div>}
    </div>
  );
}

function ItemList({
  title, icon, items, loading, empty, onRepo, seeAll,
}: {
  title: string;
  icon: React.ReactNode;
  items: (GithubItem & { repo: string })[];
  loading: boolean;
  empty: string;
  onRepo: (r: string) => void;
  seeAll?: string;
}) {
  return (
    <div className="panel flex min-w-0 max-h-[380px] flex-col">
      <div className="flex items-center justify-between border-b border-line px-5 py-3.5">
        <div className="flex items-center gap-2 text-[13.5px] text-fg">
          <span className="text-fg-3">{icon}</span> {title}
        </div>
        <div className="flex items-center gap-3">
          <span className="font-mono text-[11px] text-fg-3">{items.length}</span>
          {seeAll && (
            <Link to={seeAll} className="text-[12px] text-fg-3 hover:text-accent">
              See all →
            </Link>
          )}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading && (
          <div className="space-y-2 p-4">
            {[0, 1, 2].map((i) => <div key={i} className="skeleton h-10 rounded-lg" />)}
          </div>
        )}
        {!loading && items.length === 0 && <div className="px-5 py-8 text-center text-[13px] text-fg-3">{empty}</div>}
        {items.map((it) => (
          <a
            key={`${it.repo}#${it.number}`}
            href={it.url}
            target="_blank"
            rel="noreferrer"
            className="group flex items-start gap-3 border-b border-line px-5 py-3 last:border-0 hover:bg-white/[0.02]"
          >
            <span className="mt-0.5 font-mono text-[11px] text-fg-3">#{it.number}</span>
            <div className="min-w-0 flex-1">
              <div className="truncate text-[13.5px] group-hover:text-accent">
                {it.draft && <span className="mr-1.5 chip py-0 text-[10px]">draft</span>}
                {it.title}
              </div>
              <div className="mt-0.5 flex flex-wrap items-center gap-x-2 font-mono text-[11px] text-fg-3">
                <button
                  className="hover:text-fg"
                  onClick={(e) => {
                    e.preventDefault();
                    onRepo(it.repo);
                  }}
                >
                  {it.repo}
                </button>
                <span>·</span>
                <span>{it.author}</span>
                <span>·</span>
                <span>{timeAgo(it.updated_at)}</span>
                {it.labels.slice(0, 3).map((l) => <span key={l} className="chip py-0 text-[10px]">{l}</span>)}
              </div>
            </div>
            <ExternalLink className="mt-1 size-3 shrink-0 text-fg-3 opacity-0 group-hover:opacity-100" />
          </a>
        ))}
      </div>
    </div>
  );
}

function StatusChips({ r }: { r: Repo }) {
  const clean = r.staged + r.modified + r.untracked === 0;
  return (
    <div className="flex flex-wrap gap-1.5">
      <span className="chip">
        <GitCommitHorizontal className="size-3" /> {r.branch || "detached"}
      </span>
      {r.ahead > 0 && <span className="chip text-info"><ArrowUp className="size-3" />{r.ahead}</span>}
      {r.behind > 0 && <span className="chip text-warn"><ArrowDown className="size-3" />{r.behind}</span>}
      {clean ? (
        <span className="chip text-good">● clean</span>
      ) : (
        <>
          {r.staged > 0 && <span className="chip text-good">+{r.staged} staged</span>}
          {r.modified > 0 && <span className="chip text-warn">~{r.modified} modified</span>}
          {r.untracked > 0 && <span className="chip">?{r.untracked} untracked</span>}
        </>
      )}
      {r.stashes > 0 && <span className="chip">{r.stashes} stash</span>}
    </div>
  );
}

function RepoCard({ repo: r, gh, index, onOpen }: { repo: Repo; gh?: GithubInfo; index: number; onOpen: () => void }) {
  const last30 = sum(r.daily.slice(-30));
  return (
    <button
      onClick={onOpen}
      className="panel group flex animate-rise flex-col p-5 text-left transition hover:border-white/15 hover:bg-ink-850"
      style={{ animationDelay: `${Math.min(index, 12) * 35}ms` }}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <FolderGit2 className="size-4 text-fg-3" />
            <span className="truncate text-[16px] font-medium tracking-tight group-hover:text-accent">{r.name}</span>
          </div>
          <div className="mt-1 truncate font-mono text-[11px] text-fg-3">{r.github ?? r.remote ?? "local only"}</div>
        </div>
        {gh && !gh.error && (
          <div className="flex shrink-0 items-center gap-3 font-mono text-[11px] text-fg-2">
            <span className="flex items-center gap-1"><Star className="size-3" />{gh.stars}</span>
            <span className="flex items-center gap-1"><GitPullRequest className="size-3" />{gh.pulls.length}</span>
            <span className="flex items-center gap-1"><CircleDot className="size-3" />{gh.issues.length}</span>
          </div>
        )}
      </div>

      <div className="mt-4">
        <StatusChips r={r} />
      </div>

      <div className="mt-5">
        <div className="mb-1.5 flex items-baseline justify-between font-mono text-[10.5px] text-fg-3">
          <span>26 weeks</span>
          <span>
            <span className="text-fg-2">{last30}</span> commits · 30d
          </span>
        </div>
        <WeeklyBars daily={r.daily} height={40} />
      </div>

      <div className="mt-auto pt-4">
        {r.last_commit ? (
          <div className="flex items-center gap-2 border-t border-line pt-3 text-[12.5px]">
            <span className="truncate text-fg-2">{r.last_commit.subject}</span>
            <span className="ml-auto shrink-0 font-mono text-[11px] text-fg-3">{timeAgo(r.last_commit.timestamp)}</span>
          </div>
        ) : (
          <div className="border-t border-line pt-3 text-[12.5px] text-fg-3">{r.error ? <span className="text-bad">{r.error}</span> : "No commits yet"}</div>
        )}
      </div>
    </button>
  );
}

function RepoDrawer({ repo: r, gh, onClose }: { repo: Repo; gh?: GithubInfo; onClose: () => void }) {
  const [commits, setCommits] = useState<Commit[] | null>(null);
  useEffect(() => {
    setCommits(null);
    api.commits(r.name).then(setCommits).catch(() => setCommits([]));
  }, [r.name]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-black/50 backdrop-blur-[2px]" onClick={onClose} />
      <aside className="relative flex h-full w-full max-w-[640px] animate-rise flex-col border-l border-line bg-ink-900 shadow-2xl">
        <div className="flex items-start justify-between gap-4 border-b border-line px-6 py-5">
          <div className="min-w-0">
            <div className="eyebrow">{r.path}</div>
            <h2 className="mt-1 font-serif text-[34px] leading-none tracking-tight">{r.name}</h2>
            {gh?.description && <p className="mt-2 text-[13px] text-fg-2">{gh.description}</p>}
          </div>
          <button className="btn-ghost" onClick={onClose} aria-label="Close">
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 space-y-7 overflow-y-auto px-6 py-6">
          <StatusChips r={r} />

          {r.sites.length > 0 && <Sites sites={r.sites} />}

          <div className="grid grid-cols-4 gap-px overflow-hidden rounded-xl border border-line bg-line">
            {[
              ["Commits", fmtNum(r.total_commits)],
              ["Branches", String(r.branches)],
              ["Stars", gh && !gh.error ? String(gh.stars) : "—"],
              ["Forks", gh && !gh.error ? String(gh.forks) : "—"],
            ].map(([k, v]) => (
              <div key={k} className="bg-ink-900 px-4 py-3">
                <div className="eyebrow">{k}</div>
                <div className="mt-1 font-serif text-[28px] leading-none">{v}</div>
              </div>
            ))}
          </div>

          <div>
            <div className="eyebrow mb-3">Daily activity</div>
            <div className="overflow-x-auto"><Heatmap daily={r.daily} cell={10} gap={3} /></div>
          </div>

          {gh?.error && (
            <div className="flex items-center gap-2 rounded-xl border border-warn/20 bg-warn/[0.06] px-3 py-2 text-[12.5px] text-fg-2">
              <AlertTriangle className="size-3.5 text-warn" /> {gh.error}
            </div>
          )}
          {gh && !gh.error && (
            <div className="grid gap-5 sm:grid-cols-2">
              <MiniList title="Pull requests" items={gh.pulls} />
              <MiniList title="Issues" items={gh.issues} />
            </div>
          )}

          <div className="grid gap-6 sm:grid-cols-2">
            <div>
              <div className="eyebrow mb-3">Languages</div>
              {r.languages.length ? <ShareBars items={r.languages} /> : <div className="text-[13px] text-fg-3">—</div>}
            </div>
            <div>
              <div className="eyebrow mb-3">Contributors</div>
              <div className="space-y-1.5">
                {r.contributors.map(([name, n]) => (
                  <div key={name} className="flex items-center justify-between text-[12.5px]">
                    <span className="truncate text-fg-2">{name}</span>
                    <span className="font-mono text-[11px] text-fg-3">{n}</span>
                  </div>
                ))}
              </div>
            </div>
          </div>

          <div>
            <div className="eyebrow mb-3">Recent commits</div>
            {!commits && <div className="skeleton h-40 rounded-xl" />}
            <ol className="relative space-y-0">
              {commits?.map((c) => (
                <li key={c.hash} className="group relative flex gap-3 py-2 pl-5 before:absolute before:left-[3px] before:top-0 before:h-full before:w-px before:bg-line">
                  <span className="absolute left-0 top-[15px] size-[7px] rounded-full border border-fg-3 bg-ink-900 group-first:border-accent group-first:bg-accent" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13px]">{c.subject}</div>
                    <div className="mt-0.5 flex gap-2 font-mono text-[11px] text-fg-3">
                      <span>{c.hash.slice(0, 7)}</span>
                      <span>{c.author}</span>
                      <span>{timeAgo(c.timestamp)}</span>
                      {c.insertions !== null && <span className="text-good">+{c.insertions}</span>}
                      {c.deletions !== null && <span className="text-bad">−{c.deletions}</span>}
                    </div>
                  </div>
                </li>
              ))}
            </ol>
          </div>
        </div>
      </aside>
    </div>
  );
}

/** A project's websites (`sites.json`), grouped, each with where it goes and whether only this machine can reach it. */
function Sites({ sites }: { sites: Site[] }) {
  const groups: [string, Site[]][] = [];
  for (const s of sites) {
    const g = s.group ?? "";
    const at = groups.find(([name]) => name === g);
    if (at) at[1].push(s);
    else groups.push([g, [s]]);
  }
  return (
    <div>
      <div className="eyebrow mb-3">Sites · {sites.length}</div>
      <div className="space-y-4">
        {groups.map(([g, ss]) => (
          <div key={g}>
            {g && <div className="mb-1.5 text-[11px] font-medium uppercase tracking-wider text-fg-3">{g}</div>}
            <div className="divide-y divide-line overflow-hidden rounded-xl border border-line">
              {ss.map((s) => (
                <a
                  key={s.url}
                  href={s.url}
                  target="_blank"
                  rel="noreferrer"
                  className="group flex items-start gap-3 bg-ink-900 px-4 py-3 hover:bg-white/[0.03]"
                >
                  {s.private ? <Lock className="mt-0.5 size-4 shrink-0 text-warn" /> : <Globe className="mt-0.5 size-4 shrink-0 text-fg-3" />}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-[13.5px] font-medium group-hover:text-accent">{s.name}</span>
                      {s.private && <span className="rounded-full border border-warn/30 px-1.5 text-[10.5px] text-warn">this machine only</span>}
                    </div>
                    <div className="truncate font-mono text-[11.5px] text-fg-3">{s.url.replace(/^https?:\/\//, "")}</div>
                    {s.description && <div className="mt-1 text-[12.5px] text-fg-2">{s.description}</div>}
                  </div>
                  <ExternalLink className="mt-0.5 size-3.5 shrink-0 text-fg-3 opacity-0 group-hover:opacity-100" />
                </a>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function MiniList({ title, items }: { title: string; items: GithubItem[] }) {
  return (
    <div>
      <div className="eyebrow mb-2">{title} · {items.length}</div>
      {items.length === 0 && <div className="text-[12.5px] text-fg-3">None open.</div>}
      <div className="space-y-1">
        {items.slice(0, 8).map((i) => (
          <a key={i.number} href={i.url} target="_blank" rel="noreferrer" className="block truncate text-[12.5px] text-fg-2 hover:text-accent">
            <span className="font-mono text-fg-3">#{i.number}</span> {i.title}
          </a>
        ))}
      </div>
    </div>
  );
}
