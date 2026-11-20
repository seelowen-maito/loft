import { Fragment, useEffect, useState, type Dispatch, type KeyboardEvent, type SetStateAction } from "react";
import { FileText, Folder } from "lucide-react";
import { api, roleLabel, type Agent } from "../lib/api";
import { cx } from "../lib/format";
import { atToken, matchingEntries, mentionMode, stillFound, withPick, type Entry } from "../lib/mention";
import { type MentionGroup } from "../lib/mentionPicker";
import { Sticker } from "./Sticker";

type Pick = { agent: Agent } | { file: Entry };

/**
 * The `@` menu of a text field: while the text ends in `@…`, the `agents` whose names start with it, then files:
 * for a name, the best matches anywhere in `agentId`'s project; for a path, the entries of the folder typed so far.
 * Pass no agents for a files-only menu.
 * `onKeyDown` handles the arrows, Tab and Enter while the menu is open and returns true when it took the key.
 */
export function useMention(agentId: string, text: string, setText: Dispatch<SetStateAction<string>>, agents: Agent[] = []) {
  const token = atToken(text);
  const agentMatches = token ? agents.filter((a) => a.name.toLowerCase().startsWith(token.query)).slice(0, 6) : [];
  const mode = token && mentionMode(token.query);
  const fileDir = mode === "browse" ? token?.dir : undefined;
  const search = mode === "search" ? token?.query : undefined;
  const [listing, setListing] = useState<{ typed: string; dir: string; entries: Entry[] }>();
  useEffect(() => {
    if (fileDir === undefined) return;
    let live = true;
    api.ls(agentId, fileDir).then(
      (r) => live && setListing({ typed: fileDir, ...r }),
      () => live && setListing({ typed: fileDir, dir: "", entries: [] }),
    );
    return () => {
      live = false;
    };
  }, [agentId, fileDir]);
  const [found, setFound] = useState<{ agentId: string; typed: string; entries: Entry[] }>();
  useEffect(() => {
    if (search === undefined) return;
    let live = true;
    const t = setTimeout(
      () =>
        api.findFiles(agentId, search).then(
          (r) => live && setFound({ agentId, typed: search, entries: r.map((e) => ({ name: e.path, dir: e.dir })) }),
          () => live && setFound({ agentId, typed: search, entries: [] }),
        ),
      120,
    );
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [agentId, search]);
  // While the next search runs, the last one's results that still match stay up, so the menu doesn't blink
  // and Tab or Enter never pick one that doesn't match what's typed.
  const fileMatches =
    search !== undefined
      ? found && found.agentId === agentId
        ? stillFound(found, search)
        : []
      : token && listing && listing.typed === fileDir
        ? matchingEntries(listing.entries, token.prefix)
        : [];
  const matches: Pick[] = [...agentMatches.map((a) => ({ agent: a })), ...fileMatches.map((f) => ({ file: f }))];
  const [index, setIndex] = useState(0);
  useEffect(() => {
    setIndex(0);
  }, [text]);
  const pick = (p: Pick) => setText((t) => withPick(t, "agent" in p ? { agent: p.agent.name } : { dir: fileDir ?? "", entry: p.file }));

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
    return false;
  };

  const menu = matches.length > 0 && (
    <div
      className="max-h-80 overflow-y-auto rounded-xl border border-line-strong bg-ink-850 shadow-xl"
      role="listbox"
      aria-label={agents.length > 0 ? "Mention an agent or a file" : "Mention a file"}
    >
      {matches.map((p, i) => (
        <Fragment key={"agent" in p ? p.agent.id : p.file.name}>
          {"file" in p && i === agentMatches.length && (
            <div className={cx("px-3 pb-1 pt-1.5 font-mono text-[10.5px] text-fg-3", i > 0 && "border-t border-line")}>{search !== undefined ? "Matching files and folders" : listing?.dir ? `${listing.dir}/` : "./"}</div>
          )}
          <button
            role="option"
            aria-selected={i === index}
            ref={(el) => {
              if (i === index) el?.scrollIntoView({ block: "nearest" });
            }}
            onMouseEnter={() => setIndex(i)}
            onMouseDown={(e) => {
              e.preventDefault();
              pick(p);
            }}
            className={cx("flex w-full items-center gap-2.5 px-3 py-1.5 text-left", i === index && "bg-white/[0.06]")}
          >
            {"agent" in p ? (
              <>
                <Sticker avatar={p.agent.avatar} lead={p.agent.role === "lead"} size={20} />
                <span className="text-[13px]">{p.agent.name}</span>
                <span className="truncate text-[12px] text-fg-3">{roleLabel(p.agent.role)}</span>
              </>
            ) : (
              <>
                {p.file.dir ? <Folder className="size-4 shrink-0 text-fg-3" /> : <FileText className="size-4 shrink-0 text-fg-3" />}
                {search !== undefined ? (
                  <span className="flex min-w-0 items-baseline gap-2 font-mono">
                    <span className="shrink-0 text-[12.5px]">
                      {p.file.name.split("/").pop()}
                      {p.file.dir && "/"}
                    </span>
                    <span className="truncate text-[11px] text-fg-3">{p.file.name.split("/").slice(0, -1).join("/")}</span>
                  </span>
                ) : (
                  <span className="truncate font-mono text-[12.5px]">
                    {p.file.name}
                    {p.file.dir && "/"}
                  </span>
                )}
              </>
            )}
          </button>
          {"agent" in p && i === agentMatches.length - 1 && (
            <div className="border-t border-line px-3 py-1.5 font-mono text-[10.5px] text-fg-3">
              The message also goes to them, and their reply shows here. Start with @… to send it only to them.
            </div>
          )}
        </Fragment>
      ))}
    </div>
  );

  return { open: matches.length > 0, onKeyDown, menu };
}

/** The agents to pick after an @, grouped; `index` counts through the groups in order. `marks` says why an agent matters here. */
export function MentionMenu({ groups, index, marks, onHover, onPick }: {
  groups: MentionGroup[];
  index: number;
  marks: Record<string, string>;
  onHover: (i: number) => void;
  onPick: (a: Agent) => void;
}) {
  let i = -1;
  return (
    <div className="mb-2 max-h-72 overflow-y-auto rounded-xl border border-line-strong bg-ink-850 shadow-xl" role="listbox" aria-label="Mention an agent">
      {groups.map((g, gi) => (
        <Fragment key={`group:${g.title}`}>
          <div className={cx("px-3 pb-1 pt-1.5 font-mono text-[10.5px] text-fg-3", gi > 0 && "border-t border-line")}>{g.title}</div>
          {g.agents.map((a) => {
            const n = ++i;
            return (
              <button
                key={a.id}
                role="option"
                aria-selected={n === index}
                ref={(el) => {
                  if (n === index) el?.scrollIntoView({ block: "nearest" });
                }}
                onMouseEnter={() => onHover(n)}
                // Pointer, not click: keeps the textarea focused, and touch fires it too.
                onPointerDown={(e) => {
                  e.preventDefault();
                  onPick(a);
                }}
                className={cx("flex w-full items-center gap-2.5 px-3 py-1.5 text-left", n === index && "bg-white/[0.06]")}
              >
                <Sticker avatar={a.avatar} lead={a.role === "lead"} size={20} />
                <span className="text-[13px]">{a.name}</span>
                {marks[a.id] && <span className="rounded-full border border-line px-1.5 text-[10.5px] text-fg-2">{marks[a.id]}</span>}
                <span className="truncate text-[12px] text-fg-3">{roleLabel(a.role)}</span>
              </button>
            );
          })}
        </Fragment>
      ))}
    </div>
  );
}
