import { Fragment, useEffect, useMemo, useState, type Dispatch, type KeyboardEvent, type SetStateAction } from "react";
import { Bookmark, CircleDot, FileText, GitPullRequest } from "lucide-react";
import { api, type Agent, type SavedItem } from "../lib/api";
import { savedMatches, savedTag } from "../lib/saved";
import { cx } from "../lib/format";
import { relTime } from "../lib/time";
import { hashToken, insertTag, tagMatches, tagRows, type TagRow } from "../lib/shortcuts";
import { usePullRequests, useTasks } from "../hooks/lists";
import { IssueRow } from "./IssueRow";
import { taskStatus, toneText } from "../lib/taskStatus";

/** A row of the menu: a tag row, or a saved item. */
type Row = TagRow | { kind: "saved"; tag: string; title: string; meta: string };

const ICON = { saved: Bookmark, spec: FileText, issue: CircleDot, pr: GitPullRequest };
const TITLE = { saved: "Saved", spec: "Specs", issue: "Issues", pr: "Pull requests" };
/** A PR's icon in GitHub's colours. */
const PR_TONE: Record<string, string> = { open: "text-good", draft: "text-fg-3", merged: "text-merged", closed: "text-bad" };

/**
 * The `#` menu of a text field: while the text ends in `#…`, the specs, issues and pull requests that match, as the ⌘K
 * search finds them. `project` is the agent's: its items are tagged `#N`, the others `owner/repo#N`.
 * `onKeyDown` handles the arrows, Tab, Enter and Escape while the menu is open and returns true when it took the key.
 */
export function useTagMenu(project: string, agents: Agent[], text: string, setText: Dispatch<SetStateAction<string>>) {
  const query = hashToken(text);
  const tasks = useTasks().items;
  const prs = usePullRequests().items;
  const rows = useMemo(() => tagRows(tasks ?? [], prs ?? [], project), [tasks, prs, project]);
  const [closedAt, setClosedAt] = useState<string>();
  const [index, setIndex] = useState(0);
  useEffect(() => setIndex(0), [text]);
  // `#saved` lists the saved items, read when it is first typed.
  const [saved, setSaved] = useState<SavedItem[]>([]);
  const wantsSaved = query !== undefined && /^saved(-|$)/i.test(query);
  useEffect(() => {
    if (wantsSaved) api.savedList().then((r) => setSaved(r.items)).catch(() => {});
  }, [wantsSaved]);
  const matches: Row[] = query === undefined || closedAt === text ? [] : wantsSaved ? savedMatches(saved, query).map((i): Row => ({ kind: "saved", tag: savedTag(i), title: i.title, meta: i.tag })) : tagMatches(rows, query);
  const pick = (r: Row) => setText((t) => insertTag(t, r.tag));

  const onKeyDown = (e: KeyboardEvent) => {
    if (matches.length === 0) return false;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setIndex((i) => (i + (e.key === "ArrowDown" ? 1 : matches.length - 1)) % matches.length);
      return true;
    }
    if ((e.key === "Tab" || e.key === "Enter") && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      pick(matches[Math.min(index, matches.length - 1)]);
      return true;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      setClosedAt(text);
      return true;
    }
    return false;
  };

  const menu = matches.length > 0 && (
    <div className="max-h-80 overflow-y-auto rounded-xl border border-line-strong bg-ink-850 shadow-xl" role="listbox" aria-label="Tag a spec, issue or pull request">
      {matches.map((r, i) => r.kind === "saved" ? (
        <Fragment key={r.tag}>
          {i === 0 && <div className="px-3 pb-1 pt-1.5 font-mono text-[10.5px] text-fg-3">{TITLE.saved}</div>}
          <IssueRow icon={Bookmark} tone="text-fg-3" github={false} title={r.title} meta={[r.meta]} selected={i === index} onHover={() => setIndex(i)} onPick={() => pick(r)} />
        </Fragment>
      ) : (
        <Fragment key={`${r.kind}:${r.tag}`}>
          {r.kind !== matches[i - 1]?.kind && (
            <div className={cx("px-3 pb-1 pt-1.5 font-mono text-[10.5px] text-fg-3", i > 0 && "border-t border-line")}>{TITLE[r.kind]}</div>
          )}
          <IssueRow
            icon={ICON[r.kind]}
            tone={r.kind === "pr" ? PR_TONE[r.status] ?? "text-fg-3" : toneText(taskStatus(r.status).tone)}
            github={r.github}
            title={r.title}
            meta={[r.number !== null && `#${r.number}`, r.kind === "pr" ? r.status : taskStatus(r.status).label, agents.find((a) => a.id === r.agent)?.name, r.project, relTime(r.at)]}
            selected={i === index}
            onHover={() => setIndex(i)}
            onPick={() => pick(r)}
          />
        </Fragment>
      ))}
    </div>
  );

  return { open: matches.length > 0, onKeyDown, menu };
}
