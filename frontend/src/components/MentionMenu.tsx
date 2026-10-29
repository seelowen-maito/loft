import { Fragment } from "react";
import { type Agent } from "../lib/api";
import { type MentionGroup } from "../lib/mentionPicker";
import { cx } from "../lib/format";
import { Sticker } from "./Sticker";

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
                <Sticker avatar={a.avatar} lead={a.lead} size={20} />
                <span className="text-[13px]">{a.name}</span>
                {marks[a.id] && <span className="rounded-full border border-line px-1.5 text-[10.5px] text-fg-2">{marks[a.id]}</span>}
                <span className="truncate text-[12px] text-fg-3">{a.title}</span>
              </button>
            );
          })}
        </Fragment>
      ))}
    </div>
  );
}
