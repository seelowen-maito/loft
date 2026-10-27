import { useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { cx } from "../lib/format";
import { loadCollapsed, toggleCollapsed } from "../lib/collapsed";

/** Which project groups are folded, remembered per browser under `key`. */
export function useCollapsed(key: string) {
  const [collapsed, setCollapsed] = useState<string[]>(() => loadCollapsed(key));
  const toggle = (project: string) => setCollapsed((c) => toggleCollapsed(key, c, project));
  return { isOpen: (project: string) => !collapsed.includes(project), toggle };
}

/** The header that folds and unfolds a project's group: chevron, name, then `children` (counts) on the right. */
export function ProjectHeader({ name, open, onToggle, className, children }: { name: string; open: boolean; onToggle: () => void; className?: string; children?: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className={cx("flex w-full items-center gap-1.5 rounded-lg px-2.5 pb-0.5 pt-3 text-left text-fg-3 transition hover:text-fg", className)}
    >
      <ChevronRight className={cx("size-3 shrink-0 transition-transform", open && "rotate-90")} />
      <span className="eyebrow truncate">{name}</span>
      <span className="ml-auto flex items-center gap-2 font-mono text-[10.5px]">{children}</span>
    </button>
  );
}
