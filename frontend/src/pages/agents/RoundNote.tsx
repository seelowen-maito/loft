import { useEffect, useState } from "react";
import { MessageSquarePlus } from "lucide-react";
import { api, type GrillSession } from "../../lib/api";
import { cx } from "../../lib/format";
import { NOTE_DRAFT, clearAnswerDrafts, loadAnswerDraft } from "../../lib/draft";
import { AttachmentTile } from "../../components/AttachmentTile";
import { ErrorLine } from "../../components/Callout";
import { announceField, fieldKey, onFieldSet, useDraftField } from "./useDraftField";

/** The notes with images still uploading: Send round waits for them. */
const uploading = new Set<string>();

/** The open round's note Draft as saved, kept current. */
export function useNoteDraft(session: string, round: number | null) {
  const read = () => (round == null ? { text: "", images: [] } : loadAnswerDraft(session, round, NOTE_DRAFT));
  const [note, setNote] = useState(read);
  const key = round == null ? "" : fieldKey(session, round, NOTE_DRAFT);
  useEffect(() => {
    setNote(read());
    return onFieldSet(key, () => setNote(read()));
  }, [key]);
  return note;
}

/** Sends the open round with its Round note, if one was written, and clears the note's Draft. */
export async function sendRound(s: GrillSession) {
  if (s.round == null) return;
  const key = fieldKey(s.session_id, s.round, NOTE_DRAFT);
  if (uploading.has(key)) throw new Error("Wait for the note's images to upload.");
  const d = loadAnswerDraft(s.session_id, s.round, NOTE_DRAFT);
  const note = d.text.trim() || d.images.length > 0 ? { text: d.text.trim(), images: d.images } : undefined;
  await api.grillCommand(s.session_id, "send", undefined, note);
  clearAnswerDrafts(s.session_id, s.round, [NOTE_DRAFT]);
  announceField(key);
}

/**
 * "Anything else?": the open round's Round note, for ideas or questions its questions don't cover. Text, images
 * and @ mentions, like an answer card; it is kept as the round's Draft and goes with it on Send round, wherever
 * that is pressed (this round, the round bar, the full screen). `card` frames it like a question card.
 */
export function RoundNote({ agentId, sessionId, round, card = false }: { agentId: string; sessionId: string; round: number; card?: boolean }) {
  const { text, setText, attachments, pending, error, removeImage, paste, drop, field, mention } = useDraftField(agentId, sessionId, round, NOTE_DRAFT, "note");
  const key = fieldKey(sessionId, round, NOTE_DRAFT);
  useEffect(() => {
    if (!pending) return;
    uploading.add(key);
    return () => void uploading.delete(key);
  }, [pending, key]);

  return (
    <div {...drop.handlers} className={cx("transition", card ? "rounded-xl border bg-ink-900/70 p-3" : "rounded-lg", card && (drop.dragging ? "border-dashed border-accent/60" : "border-line"), !card && drop.dragging && "outline-dashed outline-1 outline-accent/60")}>
      <label className="mb-1.5 flex items-center gap-1.5 text-[13px] text-fg-2">
        <MessageSquarePlus className="size-3.5 shrink-0 text-accent" />
        <span>Anything else?</span>
        <span className="truncate text-[12px] text-fg-3">Ideas or questions this round doesn't cover: the agent asks about them next round.</span>
      </label>
      {attachments.length > 0 && (
        <div className="mb-2 flex flex-wrap gap-2">
          {attachments.map((a) => <AttachmentTile key={a.key} a={a} onRemove={() => removeImage(a.key)} onMissing={() => removeImage(a.key)} />)}
        </div>
      )}
      {mention.menu && <div className="mb-2">{mention.menu}</div>}
      <textarea
        ref={field}
        rows={1}
        className="input resize-none py-1.5 leading-5"
        placeholder="Optional: goes with the round when you send it"
        value={text}
        onChange={(e) => setText(e.target.value)}
        // Only the @ menu takes keys here: Enter adds a line, since sending is the round's button.
        onKeyDown={(e) => void mention.onKeyDown(e)}
        onPaste={paste}
        aria-label="Round note: anything else?"
      />
      <ErrorLine error={error} className="mt-1.5" />
    </div>
  );
}
