import { useEffect, useState } from "react";
import { Folder } from "lucide-react";
import { api, type FolderSuggestion } from "../lib/api";
import { cx } from "../lib/format";
import { folderKey, repoCount, stillMatching } from "../lib/folderSuggest";

/**
 * The Projects folder field: while the user types, the folders in the parent of what's typed and the ones found by
 * that name in their home, each with its repo count. ↑↓ move, Tab or Enter fill in the highlighted path, Esc closes.
 */
export function FolderField({ id, value, onChange }: { id?: string; value: string; onChange: (v: string) => void }) {
  // Only once the user types: not for the saved value on load, nor right after a pick.
  const [typing, setTyping] = useState(false);
  const [found, setFound] = useState<{ q: string; list: FolderSuggestion[] }>();
  const [index, setIndex] = useState(0);
  useEffect(() => {
    if (!typing || !value.trim()) return;
    let live = true;
    const t = setTimeout(
      () =>
        api.suggestFolders(value).then(
          (list) => live && setFound({ q: value, list }),
          () => live && setFound({ q: value, list: [] }),
        ),
      150,
    );
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [typing, value]);
  // While the next answer comes, the last one's folders that still fit stay up, so the list doesn't blink and Tab or
  // Enter never fill in one that doesn't match what's typed.
  const list = typing && value.trim() && found ? stillMatching(found.q, value, found.list) : [];
  const pick = (f: FolderSuggestion) => {
    onChange(f.path);
    setTyping(false);
  };

  return (
    <div className="relative">
      <input
        id={id}
        className="input font-mono"
        value={value}
        role="combobox"
        aria-expanded={list.length > 0}
        aria-autocomplete="list"
        onChange={(e) => {
          onChange(e.target.value);
          setTyping(true);
          setIndex(0);
        }}
        onBlur={() => setTyping(false)}
        onKeyDown={(e) => {
          const k = folderKey(e.key, index, list.length, e.shiftKey);
          if (!k || e.nativeEvent.isComposing) return;
          e.preventDefault();
          if (k === "close") setTyping(false);
          else if ("move" in k) setIndex(k.move);
          else pick(list[k.pick]);
        }}
      />
      {list.length > 0 && (
        <div className="absolute inset-x-0 top-full z-20 mt-1 max-h-72 overflow-y-auto rounded-xl border border-line-strong bg-ink-850 shadow-xl" role="listbox" aria-label="Folders">
          {list.map((f, i) => (
            <button
              key={f.path}
              type="button"
              role="option"
              aria-selected={i === index}
              ref={(el) => {
                if (i === index) el?.scrollIntoView({ block: "nearest" });
              }}
              onMouseEnter={() => setIndex(i)}
              // Pointer, not click: keeps the field focused, and touch fires it too.
              onPointerDown={(e) => {
                e.preventDefault();
                pick(f);
              }}
              className={cx("flex w-full items-center gap-2.5 px-3 py-1.5 text-left", i === index && "bg-white/[0.06]")}
            >
              <Folder className="size-4 shrink-0 text-fg-3" />
              <span className="min-w-0 flex-1 truncate font-mono text-[12.5px]">{f.label}</span>
              <span className={cx("shrink-0 text-[11.5px]", f.repos > 0 ? "text-fg-2" : "text-fg-3")}>{repoCount(f.repos)}</span>
            </button>
          ))}
          <div className="border-t border-line px-3 py-1.5 font-mono text-[10.5px] text-fg-3">↑↓ move · Tab or ↵ fills it in · esc closes</div>
        </div>
      )}
    </div>
  );
}
