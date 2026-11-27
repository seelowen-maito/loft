import { useEffect, useMemo, useRef, useState } from "react";
import { useLocation } from "react-router";
import { BookOpen, CircleHelp, ExternalLink, Keyboard, Mic, Search, SquareSlash, X } from "lucide-react";
import glossaryMd from "../../../GLOSSARY.md?raw";
import { api, type AgentCommand } from "../lib/api";
import { useAgents } from "../hooks/lists";
import { cx } from "../lib/format";
import { parseGlossary } from "../lib/glossary";
import { formatCombo, loadCombos, openShortcut } from "../lib/shortcuts";
import { openTalk } from "../lib/talk";
import { useOffered } from "../hooks/useOffered";
import { Dialog } from "./Dialog";
import { Tip } from "./HoverTip";

const REPO = "https://github.com/get-loft/loft";

/** The top bar's Help (?): Talk (the top bar has no mic), shortcuts, chat commands, search, the glossary and the repo. On a phone, where the live dot
 *  doesn't fit, it also says whether the event stream is connected. */
export function HelpMenu({ live }: { live: boolean }) {
  const [open, setOpen] = useState(false);
  const [shown, setShown] = useState<"commands" | "glossary" | null>(null);
  const box = useRef<HTMLDivElement>(null);
  // Talk is a Mac-only feature: its row shows only where the server offers it.
  const { voice } = useOffered();
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);
  const pick = (f: () => void) => () => {
    setOpen(false);
    f();
  };
  const row = "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] text-fg hover:bg-white/5 focus-visible:bg-white/5 focus-visible:outline-none";
  // A row's own shortcut; hidden on touch, where keys mean nothing.
  const kbd = (k: string) => <kbd className="ml-auto rounded border border-b-2 border-line-strong bg-ink-850 px-1.5 font-mono text-[10.5px] text-fg-2 [@media(pointer:coarse)]:hidden">{k}</kbd>;
  return (
    <div ref={box} className="relative">
      <Tip name="Help" off={open}>
        <button
          type="button"
          aria-label="Help"
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className={cx("rounded-full p-1.5 transition-colors", open ? "bg-white/[0.08] text-fg" : "text-fg-3 hover:text-fg")}
        >
          <CircleHelp className="size-4" />
        </button>
      </Tip>
      {open && (
        <div role="menu" aria-label="Help" className="absolute right-0 top-full z-50 mt-2 flex w-64 flex-col rounded-lg border border-line-strong bg-ink-800/95 p-1 shadow-xl backdrop-blur">
          <div className="flex items-center gap-2 px-2.5 py-1.5 text-[12.5px] text-fg-2 sm:hidden">
            <span className={cx("size-1.5 rounded-full", live ? "bg-good" : "bg-bad")} />
            {live ? "Connected" : "Reconnecting…"}
          </div>
          <hr className="my-1 border-line sm:hidden" />
          {voice && (
            <button role="menuitem" autoFocus className={row} onClick={pick(() => openTalk())}>
              <Mic className="size-3.5 text-fg-3" /> Talk to an agent {kbd("T")}
            </button>
          )}
          <button role="menuitem" autoFocus={!voice} className={row} onClick={pick(() => openShortcut("sheet"))}>
            <Keyboard className="size-3.5 text-fg-3" /> Keyboard shortcuts {kbd("?")}
          </button>
          <button role="menuitem" className={row} onClick={pick(() => setShown("commands"))}>
            <SquareSlash className="size-3.5 text-fg-3" /> Chat commands
          </button>
          <button role="menuitem" className={row} onClick={pick(() => openShortcut("switcher"))}>
            <Search className="size-3.5 text-fg-3" /> Search and jump {kbd(formatCombo(loadCombos().switcher))}
          </button>
          <hr className="my-1 border-line" />
          <button role="menuitem" className={row} onClick={pick(() => setShown("glossary"))}>
            <BookOpen className="size-3.5 text-fg-3" /> Glossary <small className="ml-auto font-mono text-[11px] text-fg-3">terms Loft uses</small>
          </button>
          <a role="menuitem" className={row} href={REPO} target="_blank" rel="noreferrer" onClick={() => setOpen(false)}>
            <ExternalLink className="size-3.5 text-fg-3" /> Loft on GitHub <span className="ml-auto text-fg-3">↗</span>
          </a>
        </div>
      )}
      {shown === "commands" && <CommandsDialog onClose={() => setShown(null)} />}
      {shown === "glossary" && <GlossaryDialog onClose={() => setShown(null)} />}
    </div>
  );
}

function Head({ title, onClose }: { title: string; onClose: () => void }) {
  return (
    <div className="mb-3 flex items-center justify-between">
      <h2 className="text-[15px] font-medium tracking-tight">{title}</h2>
      <button autoFocus className="btn-ghost" onClick={onClose} aria-label="Close"><X className="size-4" /></button>
    </div>
  );
}

/** The slash commands of the agent on screen, or of the first agent: Loft's own come first and are the same for all. */
function CommandsDialog({ onClose }: { onClose: () => void }) {
  const { pathname } = useLocation();
  const agents = useAgents().items;
  const id = /^\/agents\/([^/]+)/.exec(pathname)?.[1] ?? agents?.[0]?.id;
  const name = agents?.find((a) => a.id === id)?.name;
  const [list, setList] = useState<AgentCommand[] | null>(null);
  useEffect(() => {
    if (!id) return;
    let stale = false;
    api.commands(id).then((l) => !stale && setList(l)).catch(() => !stale && setList([]));
    return () => { stale = true; };
  }, [id]);
  return (
    <Dialog title="Chat commands" top onClose={onClose} className="p-5">
      <Head title="Chat commands" onClose={onClose} />
      <p className="mb-3 text-[12.5px] text-fg-3">Type <kbd className="font-mono text-fg-2">/</kbd> in a chat's message box to pick one.{name && ` These are ${name}'s.`}</p>
      {!id ? (
        <p className="text-[13px] text-fg-2">Create an agent to see its commands.</p>
      ) : !list ? (
        <div className="skeleton h-24 rounded-lg" />
      ) : (
        <ul className="max-h-[60vh] space-y-1.5 overflow-y-auto">
          {list.map((c) => (
            <li key={c.name} className="flex gap-3 text-[13px]">
              <code className="shrink-0 font-mono text-[12.5px] text-accent">{c.name}</code>
              <span className="min-w-0 truncate text-fg-2" title={c.description}>{c.description}</span>
            </li>
          ))}
        </ul>
      )}
    </Dialog>
  );
}

/** GLOSSARY.md's terms, by section, built into the app. */
function GlossaryDialog({ onClose }: { onClose: () => void }) {
  const terms = useMemo(() => parseGlossary(glossaryMd), []);
  const sections = [...new Set(terms.map((t) => t.section))];
  return (
    <Dialog title="Glossary" top onClose={onClose} className="p-5">
      <Head title="Glossary" onClose={onClose} />
      <div className="max-h-[65vh] space-y-5 overflow-y-auto pr-1">
        {sections.map((s) => (
          <section key={s}>
            <h3 className="mb-2 font-mono text-[10.5px] uppercase tracking-[0.14em] text-fg-3">{s}</h3>
            <dl className="space-y-3">
              {terms.filter((t) => t.section === s).map((t) => (
                <div key={t.term}>
                  <dt className="text-[13px] font-medium text-fg">{t.term}</dt>
                  <dd className="text-[12.5px] text-fg-2">{t.meaning}</dd>
                  {t.avoid && <dd className="text-[11.5px] text-fg-3">Avoid: {t.avoid}</dd>}
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
    </Dialog>
  );
}
