import { useState } from "react";
import { Bookmark } from "lucide-react";
import { api } from "../../lib/api";
import { cx } from "../../lib/format";
import { defaultTitle, SUGGESTED_TAGS } from "../../lib/saved";
import { useAction } from "../../lib/useAction";
import { Dialog } from "../../components/Dialog";
import { ActionButton } from "../../components/ActionButton";
import { ErrorLine } from "../../components/Callout";
import { toast } from "../../components/FloatingBar";

/** Save in a message's footer: a small form for the title and tag, then the message is kept in Saved. */
export function SaveButton({ agentId, m }: { agentId: string; m: { id: number; content: string } }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)} className="msg-act" aria-label="Save this message" title="Save this message">
        <Bookmark className="size-3.5" aria-hidden /> Save
      </button>
      {open && <SaveDialog agentId={agentId} m={m} onClose={() => setOpen(false)} />}
    </>
  );
}

export function SaveDialog({ agentId, m, onClose }: { agentId: string; m: { id: number; content: string }; onClose: () => void }) {
  const [title, setTitle] = useState(() => defaultTitle(m.content));
  const [tag, setTag] = useState(SUGGESTED_TAGS[0]);
  const save = useAction();
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const item = await save.run(() => api.saveMessage(agentId, m.id, title.trim(), tag.trim()));
    if (!item) return;
    toast(`Saved “${item.title}”`);
    onClose();
  };
  return (
    <Dialog title="Save message" onClose={onClose} size="sm">
      <form onSubmit={submit} className="flex flex-col gap-3 p-5">
        <h2 className="text-[15px] font-medium">Save message</h2>
        <label className="flex flex-col gap-1 text-[12px] text-fg-3">
          Title
          <input className="input" value={title} onChange={(e) => setTitle(e.target.value)} autoFocus aria-label="Title" />
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
          <input className="input" value={tag} onChange={(e) => setTag(e.target.value)} placeholder="Or type a tag" aria-label="Tag" />
        </div>
        <ErrorLine error={save.error} />
        <div className="flex justify-end gap-2">
          <button type="button" className="btn-ghost" onClick={onClose}>Cancel</button>
          <ActionButton type="submit" pending={save.busy} pendingLabel="Saving…" disabled={!title.trim()} icon={<Bookmark className="size-3.5" />}>
            Save
          </ActionButton>
        </div>
      </form>
    </Dialog>
  );
}
