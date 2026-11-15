import { useEffect, useRef } from "react";
import { CircleDot, type LucideIcon } from "lucide-react";
import { Origin } from "./Origin";
import { cx } from "../lib/format";

/** One option of the issue picker and the `#` menu: a status-toned icon, where it lives, the title, then
 * "#N · status · agent · repo · age" (the empty parts left out). The selected row scrolls into view. */
export function IssueRow({ icon: Icon = CircleDot, tone, github, title, meta, selected, onHover, onPick }: {
  icon?: LucideIcon;
  tone: string;
  github: boolean;
  title: string;
  meta: (string | null | undefined | false)[];
  selected: boolean;
  onHover: () => void;
  onPick: () => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (selected) ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [selected]);
  return (
    <button
      ref={ref}
      role="option"
      aria-selected={selected}
      onMouseEnter={onHover}
      onMouseDown={(e) => {
        e.preventDefault();
        onPick();
      }}
      className={cx("flex w-full items-center gap-2.5 px-3 py-1.5 text-left", selected && "bg-white/[0.06]")}
    >
      <Icon className={cx("size-3.5 shrink-0", tone)} />
      <Origin github={github} />
      <span className="min-w-0 flex-1 truncate text-[13px]">{title}</span>
      <span className="min-w-0 max-w-[50%] truncate font-mono text-[10.5px] text-fg-3">{meta.filter(Boolean).join(" · ")}</span>
    </button>
  );
}
