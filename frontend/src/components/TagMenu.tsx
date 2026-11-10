import { useEffect, useMemo, useState, type Dispatch, type KeyboardEvent, type SetStateAction } from "react";
import { CircleDot, FileText, GitPullRequest } from "lucide-react";
import { api, type PullRequest } from "../lib/api";
import { cx } from "../lib/format";
import { hashToken, insertTag, tagMatches, tagRows, type TagRow } from "../lib/shortcuts";
import { useTasks } from "./TaskChip";

const ICON = { spec: FileText, issue: CircleDot, pr: GitPullRequest };

/**
 * The `#` menu of a text field: while the text ends in `#…`, the specs, issues and pull requests that match, as the ⌘K
 * search finds them. `project` is the agent's: its items are tagged `#N`, the others `owner/repo#N`.
 * `onKeyDown` handles the arrows, Tab, Enter and Escape while the menu is open and returns true when it took the key.
 */
export function useTagMenu(project: string, text: string, setText: Dispatch<SetStateAction<string>>) {
  const query = hashToken(text);
  const tasks = useTasks();
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const wants = query !== undefined;
  useEffect(() => {
    if (wants) api.pullRequests().then(setPrs, () => {});
  }, [wants]);
  const rows = useMemo(() => tagRows(tasks ?? [], prs, project), [tasks, prs, project]);
  const [closedAt, setClosedAt] = useState<string>();
  const [index, setIndex] = useState(0);
  useEffect(() => setIndex(0), [text]);
  const matches = query === undefined || closedAt === text ? [] : tagMatches(rows, query);
  const pick = (r: TagRow) => setText((t) => insertTag(t, r.tag));

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
      {matches.map((r, i) => {
        const Icon = ICON[r.kind];
        return (
          <button
            key={`${r.kind}:${r.tag}`}
            role="option"
            aria-selected={i === index}
            ref={(el) => {
              if (i === index) el?.scrollIntoView({ block: "nearest" });
            }}
            onMouseEnter={() => setIndex(i)}
            onMouseDown={(e) => {
              e.preventDefault();
              pick(r);
            }}
            className={cx("flex w-full items-center gap-2.5 px-3 py-1.5 text-left", i === index && "bg-white/[0.06]")}
          >
            <Icon className={cx("size-4 shrink-0", r.open ? "text-fg-2" : "text-fg-3")} />
            <span className="min-w-0 flex-1 truncate text-[13px]">{r.title}</span>
            <span className="shrink-0 font-mono text-[11px] text-fg-3">{r.number ? `#${r.number}` : r.kind} · {r.project || "no project"}</span>
          </button>
        );
      })}
    </div>
  );

  return { open: matches.length > 0, onKeyDown, menu };
}
