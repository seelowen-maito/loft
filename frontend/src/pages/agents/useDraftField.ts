import { useEffect, useId, useRef, useState, type ClipboardEvent } from "react";
import { loadAnswerDraft, saveAnswerDraft, type AnswerDraft } from "../../lib/draft";
import { draftImages, type Attachment } from "../../lib/useDraft";
import { attachImages, pastedImages, useFileDrop } from "../../lib/attach";
import { useAutoGrow } from "../../lib/autoGrow";
import { useMention } from "../../components/MentionMenu";

/** Sent on `window` when a grill field's Draft changed, so the same field shown elsewhere (the chat's round, the full screen) shows it too. */
const FIELD_SET = "loft:grill-field-set";
type Changed = { key: string; from: string };

/** A grill field's name in `FIELD_SET`: its Draft's session, round and question (or `NOTE_DRAFT`). */
export const fieldKey = (session: string, round: number, key: string) => `${session}:${round}:${key}`;

/** Tells every field showing this Draft that it changed; `from` is the field that changed it, which ignores it. */
export function announceField(key: string, from = "") {
  window.dispatchEvent(new CustomEvent<Changed>(FIELD_SET, { detail: { key, from } }));
}

/** Calls `f` whenever the Draft of the field `key` changes; returns the unsubscribe. */
export function onFieldSet(key: string, f: (from: string) => void) {
  const on = (e: Event) => {
    const d = (e as CustomEvent<Changed>).detail;
    if (d.key === key) f(d.from);
  };
  window.addEventListener(FIELD_SET, on);
  return () => window.removeEventListener(FIELD_SET, on);
}

const stored = (d: AnswerDraft) => `${d.text}\n${d.images.join("\n")}`;
const sameDraft = (a: AnswerDraft, b: AnswerDraft) => stored(a) === stored(b);

/**
 * A grill field of text and images (an answer card's, the Round note): kept as the round's Draft under `key`, so
 * it survives switching agents or tabs and shows the same wherever the field is on screen. Images are pasted or
 * dropped (on the field's own box), files picked with "@", and the textarea grows up to 5 lines.
 *
 * `saved` is what was already sent (a saved answer): shown when there's no Draft, the Draft is emptied while the
 * field matches it, and when it changes (an answer given in the app) the field shows the new one.
 */
export function useDraftField(agentId: string, sessionId: string, round: number, key: string, kind: string, saved?: AnswerDraft) {
  const me = useId();
  const name = fieldKey(sessionId, round, key);
  const savedRef = useRef(saved);
  savedRef.current = saved;
  const shown = (d: AnswerDraft) => (d.text || d.images.length > 0 ? d : (savedRef.current ?? d));
  const [initial] = useState(() => loadAnswerDraft(sessionId, round, key));
  const [text, setText] = useState(() => shown(initial).text);
  const [attachments, setAttachments] = useState<Attachment[]>(() => draftImages(agentId, shown(initial).images));
  const [error, setError] = useState<string | null>(null);
  const images = attachments.flatMap((a) => (a.file ? [a.file] : []));
  const imageList = images.join("\n");
  const pending = attachments.some((a) => !a.file && !a.error);
  const failed = attachments.some((a) => a.error);
  const same = !!saved && saved.text === text.trim() && saved.images.join("\n") === imageList;

  // Shows a Draft or answer, keeping the tiles already shown when its images are the same.
  const show = (d: AnswerDraft) => {
    setText(d.text);
    const files = d.images.join("\n");
    setAttachments((a) => (a.map((x) => x.file).join("\n") === files ? a : (a.forEach((x) => URL.revokeObjectURL(x.preview)), draftImages(agentId, d.images))));
  };

  // Every change is saved and announced; `loaded` is what the Draft holds, so loading one doesn't save it back.
  const loaded = useRef(stored(initial));
  useEffect(() => {
    const d = same ? { text: "", images: [] } : { text, images: imageList ? imageList.split("\n") : [] };
    if (stored(d) === loaded.current) return;
    loaded.current = stored(d);
    saveAnswerDraft(sessionId, round, key, d);
    announceField(name, me);
  }, [sessionId, round, key, name, me, same, text, imageList]);
  useEffect(
    () =>
      onFieldSet(name, (from) => {
        if (from === me) return;
        const d = loadAnswerDraft(sessionId, round, key);
        loaded.current = stored(d);
        show(shown(d));
      }),
    [name, me, sessionId, round, key, agentId],
  );
  // A new saved answer (from the app) replaces the field; on mount the Draft wins.
  const savedKey = saved ? stored(saved) : null;
  const seen = useRef(saved);
  useEffect(() => {
    const was = seen.current;
    seen.current = saved;
    if (!saved || (was && sameDraft(was, saved))) return;
    show(saved);
  }, [savedKey]);

  const addImages = (files: File[]) => setError(attachImages(agentId, files, attachments.length, setAttachments, kind));
  const removeImage = (k: string) => setAttachments((a) => a.filter((x) => (x.key === k ? (URL.revokeObjectURL(x.preview), false) : true)));
  const paste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = pastedImages(e);
    if (files.length > 0) addImages(files);
  };
  // Images are dropped on the field's box, not on the chat around it.
  const drop = useFileDrop(addImages, true);
  const field = useRef<HTMLTextAreaElement>(null);
  useAutoGrow(field, text, 1, 5);
  // "@" picks a file from the agent's workspace, as in the chat; no agents, the field goes only to this one.
  const mention = useMention(agentId, text, setText);

  return { text, setText, attachments, images, pending, failed, same, error, setError, addImages, removeImage, paste, drop, field, mention };
}
