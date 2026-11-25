import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowLeft, Bookmark, FileUp, MessageSquare, Pencil, Search, Trash2 } from "lucide-react";
import { api, type PullRequest, type Repo, type SavedItem } from "../lib/api";
import { cx } from "../lib/format";
import { fmtDate, fmtDateTime } from "../lib/time";
import { errorText, useAction } from "../lib/useAction";
import { excerpt, sourceHref, SUGGESTED_TAGS, tagOptions } from "../lib/saved";
import { ErrorLine } from "../components/Callout";
import { ActionButton } from "../components/ActionButton";
import { ConfirmDialog, Dialog } from "../components/Dialog";
import { MD_COMPONENTS } from "./agents/Chat";
import { PullRequestCard } from "./agents/PullRequestCard";

const MISSING = "This saved item doesn't exist.";

const PAGE = "mx-auto flex w-full max-w-[900px] flex-col gap-4 px-4 py-5 sm:px-5";

/** Saved: every kept message, newest first, with search and a tag filter. `/saved/:id` opens one. */
export default function SavedPage() {
  const { id } = useParams();
  return id ? <ItemPage id={Number(id)} /> : <ListPage />;
}

function ListPage() {
  const [q, setQ] = useState("");
  const [tag, setTag] = useState("");
  const [list, setList] = useState<{ items: SavedItem[]; tags: string[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    // Typing searches once it pauses.
    const t = setTimeout(() => {
      api.savedList(q.trim(), tag).then((l) => live && (setList(l), setError(null)), (e) => live && setError(errorText(e)));
    }, 200);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [q, tag]);
  const tags = tagOptions(list?.tags ?? []);
  return (
    <div className={PAGE}>
      <h1 className="flex items-center gap-2 text-[18px] font-medium tracking-tight">
        <Bookmark className="size-4 text-accent" /> Saved
      </h1>
      <div className="flex flex-wrap items-center gap-2">
        <label className="flex min-w-[160px] flex-1 items-center gap-2 rounded-xl border border-line-strong bg-ink-850 px-3 py-1.5 sm:max-w-xs">
          <Search className="size-3.5 text-fg-3" />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search titles and text" aria-label="Search saved" className="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-3" />
        </label>
        <div className="flex flex-wrap items-center gap-1" role="group" aria-label="Tag">
          {["", ...tags].map((t) => (
            <button key={t || "all"} type="button" aria-pressed={tag === t} onClick={() => setTag(t)} className={cx("rounded-full px-2.5 py-0.5 text-[12px]", tag === t ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
              {t || "All"}
            </button>
          ))}
        </div>
      </div>
      <ErrorLine error={error} />
      {list && list.items.length === 0 && (
        <p className="py-10 text-center text-[13px] text-fg-3">
          {q || tag ? "Nothing saved matches." : "Nothing saved yet. Save a message from its footer in any chat."}
        </p>
      )}
      <ul className="flex flex-col gap-2">
        {list?.items.map((i) => (
          <li key={i.id}>
            <Link to={`/saved/${i.id}`} className="flex flex-col gap-1 rounded-xl border border-line bg-white/[0.02] px-4 py-3 transition-colors hover:bg-white/[0.05]">
              <div className="flex items-baseline gap-2">
                <span className="min-w-0 flex-1 truncate text-[14px] text-fg">{i.title}</span>
                {i.tag && <span className="chip shrink-0">{i.tag}</span>}
              </div>
              <span className="line-clamp-2 text-[12.5px] text-fg-3">{excerpt(i.content)}</span>
              <span className="font-mono text-[10.5px] text-fg-3">{i.agent_name} · {fmtDate(i.created_at)}</span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

type Item = SavedItem & { source_exists: boolean; pr: PullRequest | null };

function ItemPage({ id }: { id: number }) {
  const navigate = useNavigate();
  const [item, setItem] = useState<Item | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [exporting, setExporting] = useState(false);
  const remove = useAction();
  useEffect(() => {
    let live = true;
    if (!Number.isInteger(id)) return setError(MISSING);
    api.savedItem(id).then((i) => live && setItem(i), (e) => live && setError(errorText(e) === "not found" ? MISSING : errorText(e)));
    return () => {
      live = false;
    };
  }, [id]);
  return (
    <div className={PAGE}>
      <Link to="/saved" className="flex w-fit items-center gap-1 text-[12.5px] text-fg-3 hover:text-fg">
        <ArrowLeft className="size-3.5" /> Saved
      </Link>
      <ErrorLine error={error} />
      {item && editing && <EditForm item={item} onSaved={(i) => (setItem({ ...item, ...i }), setEditing(false))} onCancel={() => setEditing(false)} />}
      {item && !editing && (
        <article className="flex flex-col gap-3">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-start">
            <h1 className="min-w-0 flex-1 text-[20px] font-medium tracking-tight [overflow-wrap:anywhere]">{item.title}</h1>
            <div className="-ml-2.5 flex flex-wrap items-center gap-1 sm:ml-0">
              <button type="button" className="btn-ghost" onClick={() => setExporting(true)}>
                <FileUp className="size-3.5" aria-hidden /> Export to docs
              </button>
              <button type="button" className="btn-ghost" onClick={() => setEditing(true)}>
                <Pencil className="size-3.5" aria-hidden /> Edit
              </button>
              <button type="button" className="btn-ghost" onClick={() => setDeleting(true)}>
                <Trash2 className="size-3.5" aria-hidden /> Delete
              </button>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2 font-mono text-[11px] text-fg-3">
            {item.tag && <span className="chip">{item.tag}</span>}
            <span>From {item.agent_name} · {fmtDate(item.created_at)}</span>
            {item.updated_at && <span>· Edited {fmtDateTime(item.updated_at)}</span>}
            {item.source_exists ? (
              <Link to={sourceHref(item)} className="flex items-center gap-1 text-accent hover:underline">
                <MessageSquare className="size-3" /> Open in chat
              </Link>
            ) : (
              <span>· The source message is gone</span>
            )}
          </div>
          {item.pr && (
            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] text-fg-3">
                Exported to docs: <a href={item.pr.url} target="_blank" rel="noopener noreferrer" className="text-accent hover:underline">{item.pr.slug}#{item.pr.number}</a>
              </span>
              <PullRequestCard pr={item.pr} />
            </div>
          )}
          <Body content={item.content} />
        </article>
      )}
      {item && exporting && <ExportDialog id={item.id} onDone={(pr) => (setItem({ ...item, pr, export_pr: pr.id }), setExporting(false))} onClose={() => setExporting(false)} />}
      {item && deleting && (
        <ConfirmDialog
          title="Delete this saved item?"
          body="It's removed from Saved. The message in the chat stays. This can't be undone."
          cancelLabel="Keep it"
          confirmLabel="Delete"
          pendingLabel="Deleting…"
          danger
          pending={remove.busy}
          error={remove.error}
          onConfirm={() => remove.run(async () => { await api.deleteSaved(item.id); navigate("/saved"); })}
          onClose={() => setDeleting(false)}
        />
      )}
    </div>
  );
}

/** An item's content as it reads on its page; the edit form's preview shows the same. */
function Body({ content }: { content: string }) {
  return (
    <div className="prose-chat rounded-2xl border border-line bg-white/[0.02] px-5 py-4 [overflow-wrap:anywhere]">
      <Markdown remarkPlugins={[remarkGfm]} components={MD_COMPONENTS}>{content}</Markdown>
    </div>
  );
}

const FIELD = "w-full rounded-xl border border-line-strong bg-ink-850 px-3 py-2 text-[13px] outline-none placeholder:text-fg-3";

/** Edit: the title, the tag and the content as Markdown, with a preview. */
function EditForm({ item, onSaved, onCancel }: { item: SavedItem; onSaved: (i: SavedItem) => void; onCancel: () => void }) {
  const [title, setTitle] = useState(item.title);
  const [tag, setTag] = useState(item.tag);
  const [content, setContent] = useState(item.content);
  const [preview, setPreview] = useState(false);
  const save = useAction();
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const i = await save.run(() => api.updateSaved(item.id, title.trim(), tag.trim(), content));
    if (i) onSaved(i);
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-3" aria-label="Edit saved item">
      <label className="flex flex-col gap-1 text-[12px] text-fg-3">
        Title
        <input className={FIELD} value={title} onChange={(e) => setTitle(e.target.value)} autoFocus aria-label="Title" />
      </label>
      <div className="flex flex-col gap-1.5 text-[12px] text-fg-3">
        Tag
        <div className="flex flex-wrap gap-1.5" role="group" aria-label="Suggested tags">
          {SUGGESTED_TAGS.map((t) => (
            <button key={t} type="button" aria-pressed={tag === t} onClick={() => setTag(t)} className={cx("chip", tag === t && "border-accent/60 bg-accent/10 text-fg")}>
              {t}
            </button>
          ))}
        </div>
        <input className={FIELD} value={tag} onChange={(e) => setTag(e.target.value)} placeholder="Or type a tag" aria-label="Tag" />
      </div>
      <div className="flex flex-col gap-1.5 text-[12px] text-fg-3">
        <div className="flex items-center gap-1" role="group" aria-label="Content view">
          <span className="flex-1">Content, in Markdown</span>
          {(["Write", "Preview"] as const).map((v) => (
            <button key={v} type="button" aria-pressed={preview === (v === "Preview")} onClick={() => setPreview(v === "Preview")} className={cx("rounded-full px-2.5 py-0.5 text-[12px]", preview === (v === "Preview") ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}>
              {v}
            </button>
          ))}
        </div>
        {preview ? (
          <Body content={content} />
        ) : (
          <textarea value={content} onChange={(e) => setContent(e.target.value)} rows={16} className={cx(FIELD, "resize-y font-mono text-[12.5px]")} aria-label="Content" />
        )}
      </div>
      <ErrorLine error={save.error} />
      <div className="flex justify-end gap-2">
        <button type="button" className="btn-ghost" onClick={onCancel}>Cancel</button>
        <ActionButton type="submit" pending={save.busy} pendingLabel="Saving…" disabled={!title.trim() || !content.trim()}>
          Save
        </ActionButton>
      </div>
    </form>
  );
}

/** Export to docs: pick a project; Loft opens a pull request on its repo adding `docs/<slug>.md`. */
function ExportDialog({ id, onDone, onClose }: { id: number; onDone: (pr: PullRequest) => void; onClose: () => void }) {
  const [repos, setRepos] = useState<Repo[] | null>(null);
  const [project, setProject] = useState("");
  const load = useAction();
  const send = useAction();
  useEffect(() => {
    load.run(() => api.repos()).then((r) => {
      if (!r) return;
      setRepos(r);
      setProject((r.find((x) => x.github) ?? r[0])?.name ?? "");
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const chosen = repos?.find((r) => r.name === project);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const pr = await send.run(() => api.exportSaved(id, project));
    if (pr) onDone(pr);
  };
  return (
    <Dialog title="Export to docs" onClose={onClose} size="sm">
      <form onSubmit={submit} className="flex flex-col gap-3 p-5">
        <h2 className="text-[15px] font-medium">Export to docs</h2>
        <p className="text-[12.5px] text-fg-3">Loft opens a pull request on the project's repo adding this item as a page in its <code>docs/</code> folder, or updating it if it's there.</p>
        <label className="flex flex-col gap-1 text-[12px] text-fg-3">
          Project
          <select className="input" value={project} onChange={(e) => (setProject(e.target.value), send.setError(null))} aria-label="Project" disabled={!repos?.length}>
            {repos?.map((r) => (
              <option key={r.name} value={r.name}>{r.github ? `${r.name} (${r.github})` : `${r.name} (no GitHub repo)`}</option>
            ))}
          </select>
        </label>
        {repos && !repos.length && <p className="text-[12.5px] text-fg-3">No projects yet.</p>}
        {chosen && !chosen.github && (
          <p className="text-[12.5px] text-fg-3" role="note">{chosen.name} has no GitHub repo, so Loft can't open a pull request there. Pick another project, or push it to GitHub first.</p>
        )}
        <ErrorLine error={load.error ?? send.error} />
        <div className="flex justify-end gap-2">
          <button type="button" className="btn-ghost" onClick={onClose}>Cancel</button>
          <ActionButton type="submit" pending={send.busy} pendingLabel="Opening PR…" disabled={!chosen?.github} icon={<FileUp className="size-3.5" />}>
            Open PR
          </ActionButton>
        </div>
      </form>
    </Dialog>
  );
}
