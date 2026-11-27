import { useEffect, useId, useRef, useState } from "react";
import { MessageSquarePlus } from "lucide-react";
import { api, type GrillSession } from "../../lib/api";
import { cx } from "../../lib/format";
import { NOTE_DRAFT, clearAnswerDrafts, loadAnswerDraft, saveAnswerDraft } from "../../lib/draft";
import { draftImages, type Attachment } from "../../lib/useDraft";
import { attachImages, pastedImages, useFileDrop } from "../../lib/attach";
import { AttachmentTile } from "../../components/AttachmentTile";
import { useAutoGrow } from "../../lib/autoGrow";
import { useMention } from "../../components/MentionMenu";
import { ErrorLine } from "../../components/Callout";

/** Sent on `window` when a round's note Draft changed, so its other fields (the chat's round, the full screen) and the round bar show it. */
const NOTE_SET = "loft:round-note-set";
const noteKey = (session: string, round: number) => `${session}:${round}`;
/** The notes with images still uploading: Send round waits for them. */
const uploading = new Set<string>();

type Changed = { key: string; from: string };

function announce(key: string, from: string) {
  window.dispatchEvent(new CustomEvent<Changed>(NOTE_SET, { detail: { key, from } }));
}

/** The open round's note Draft as saved, kept current. */
export function useNoteDraft(session: string, round: number | null) {
  const read = () => (round == null ? { text: "", images: [] } : loadAnswerDraft(session, round, NOTE_DRAFT));
  const [note, setNote] = useState(read);
  const key = round == null ? "" : noteKey(session, round);
  useEffect(() => {
    setNote(read());
    const onSet = (e: Event) => {
      if ((e as CustomEvent<Changed>).detail.key === key) setNote(read());
    };
    window.addEventListener(NOTE_SET, onSet);
    return () => window.removeEventListener(NOTE_SET, onSet);
  }, [key]);
  return note;
}

/** Sends the open round with its Round note, if one was written, and clears the note's Draft. */
export async function sendRound(s: GrillSession) {
  if (s.round == null) return;
  const key = noteKey(s.session_id, s.round);
  if (uploading.has(key)) throw new Error("Wait for the note's images to upload.");
  const d = loadAnswerDraft(s.session_id, s.round, NOTE_DRAFT);
  const note = d.text.trim() || d.images.length > 0 ? { text: d.text.trim(), images: d.images } : undefined;
  await api.grillCommand(s.session_id, "send", undefined, note);
  clearAnswerDrafts(s.session_id, s.round, [NOTE_DRAFT]);
  announce(key, "");
}

/**
 * "Anything else?": the open round's Round note, for ideas or questions its questions don't cover. Text, images
 * and @ mentions, like an answer card; it is kept as the round's Draft and goes with it on Send round, wherever
 * that is pressed (this round, the round bar, the full screen). `card` frames it like a question card.
 */
export function RoundNote({ agentId, sessionId, round, card = false }: { agentId: string; sessionId: string; round: number; card?: boolean }) {
  const key = noteKey(sessionId, round);
  const me = useId();
  const [initial] = useState(() => loadAnswerDraft(sessionId, round, NOTE_DRAFT));
  const [text, setText] = useState(initial.text);
  const [attachments, setAttachments] = useState<Attachment[]>(() => draftImages(agentId, initial.images));
  const [attachErr, setAttachErr] = useState<string | null>(null);
  const images = attachments.flatMap((a) => (a.file ? [a.file] : []));
  const imageList = images.join("\n");
  const pending = attachments.some((a) => !a.file && !a.error);

  // Every change is the round's note Draft; the other fields showing this note pick it up.
  const loaded = useRef(`${initial.text}\n${initial.images.join("\n")}`);
  useEffect(() => {
    const now = `${text}\n${imageList}`;
    if (now === loaded.current) return;
    loaded.current = now;
    saveAnswerDraft(sessionId, round, NOTE_DRAFT, { text, images: imageList ? imageList.split("\n") : [] });
    announce(key, me);
  }, [sessionId, round, key, me, text, imageList]);
  useEffect(() => {
    const onSet = (e: Event) => {
      const d = (e as CustomEvent<Changed>).detail;
      if (d.key !== key || d.from === me) return;
      const n = loadAnswerDraft(sessionId, round, NOTE_DRAFT);
      loaded.current = `${n.text}\n${n.images.join("\n")}`;
      setText(n.text);
      setAttachments((a) => (a.map((x) => x.file).join("\n") === n.images.join("\n") ? a : (a.forEach((x) => URL.revokeObjectURL(x.preview)), draftImages(agentId, n.images))));
    };
    window.addEventListener(NOTE_SET, onSet);
    return () => window.removeEventListener(NOTE_SET, onSet);
  }, [key, me, sessionId, round, agentId]);
  useEffect(() => {
    if (!pending) return;
    uploading.add(key);
    return () => void uploading.delete(key);
  }, [pending, key]);

  const addImages = (files: File[]) => setAttachErr(attachImages(agentId, files, attachments.length, setAttachments, "note"));
  const removeImage = (k: string) => setAttachments((a) => a.filter((x) => (x.key === k ? (URL.revokeObjectURL(x.preview), false) : true)));
  const drop = useFileDrop(addImages, true);
  const field = useRef<HTMLTextAreaElement>(null);
  useAutoGrow(field, text, 1, 5);
  // "@" picks a file from the agent's workspace, as on the cards.
  const mention = useMention(agentId, text, setText);

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
        onPaste={(e) => {
          const files = pastedImages(e);
          if (files.length > 0) addImages(files);
        }}
        aria-label="Round note: anything else?"
      />
      <ErrorLine error={attachErr} className="mt-1.5" />
    </div>
  );
}
