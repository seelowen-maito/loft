import { useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, CornerDownLeft, MessageCircle, Send, Sparkles, Wand2 } from "lucide-react";
import { api, grillError, type GrillQuestion, type GrillSession, type Message } from "../../lib/api";
import { cx } from "../../lib/format";
import { NOTE_DRAFT, clearAnswerDrafts, loadAnswerDraft, saveAnswerDraft } from "../../lib/draft";
import { draftImages, type Attachment } from "../../lib/useDraft";
import { attachImages, pastedImages, useFileDrop } from "../../lib/attach";
import { AttachmentTile } from "../../components/AttachmentTile";
import { useAutoGrow } from "../../lib/autoGrow";
import { useMention } from "../../components/MentionMenu";
import { questionAnchor } from "../../lib/interview";
import { ActionButton } from "../../components/ActionButton";
import { ErrorLine } from "../../components/Callout";
import { useAction } from "../../lib/useAction";
import { RoundNote, sendRound } from "./RoundNote";

type Posted = Pick<GrillQuestion, "id" | "text" | "recommendation" | "from_round">;

/**
 * A round the agent posted (`meta.grill.role == "round"`). The round waiting for answers shows as cards
 * with Accept and an answer field, fed by the live session; earlier rounds show their questions read-only.
 */
export function GrillRound({ m, agentId, session }: { m: Message; agentId: string; session: GrillSession | null }) {
  const g = m.meta?.grill ?? {};
  const open = !!session && session.session_id === g.session_id && session.round === g.round && !session.waiting && session.state !== "ended";
  const posted: Posted[] = g.questions ?? [];
  const live = open ? (session?.questions ?? []) : [];
  const sending = useAction();
  // A round that was sent, ended or followed by another keeps no answer Drafts. Unknown while the session loads.
  const closed = !!session && !open;
  const postedIds = posted.map((q) => q.id).join("\n");
  useEffect(() => {
    if (closed && g.session_id && g.round != null) clearAnswerDrafts(g.session_id, g.round, [...postedIds.split("\n"), NOTE_DRAFT]);
  }, [closed, g.session_id, g.round, postedIds]);

  const send = () => {
    if (!session) return;
    return sending.run(() => sendRound(session));
  };

  const answered = live.filter((q) => q.state !== "open").length;
  return (
    <div className="pl-9">
      <div className={cx("rounded-2xl border p-4", open ? "border-accent/25 bg-accent/[0.04]" : "border-line bg-ink-900/60")}>
        <div className="mb-3 flex items-center gap-2 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">
          <span className={open ? "text-accent" : undefined}>Round {g.round}</span>
          {g.topic && <span className="truncate normal-case tracking-normal">· {g.topic}</span>}
        </div>
        <div className="space-y-3">
          {open
            ? live.map((q, i) => <QuestionCard key={q.id} n={i + 1} q={q} agentId={agentId} sessionId={session!.session_id} round={g.round} />)
            : posted.map((q, i) => (
                <div key={q.id} className="text-[13.5px]">
                  <div className="prose-chat text-fg-2">
                    <Markdown remarkPlugins={[remarkGfm]}>{`${i + 1}. ${q.text}`}</Markdown>
                  </div>
                  <div className="mt-0.5 pl-4 text-[12.5px] text-fg-3">Recommended: {q.recommendation}</div>
                </div>
              ))}
        </div>
        {open && (
          <div className="mt-4 border-t border-line pt-3">
            <RoundNote agentId={agentId} sessionId={session!.session_id} round={g.round} />
          </div>
        )}
        {open && (
          <div className="mt-3 flex items-center gap-3">
            <span className="flex-1 font-mono text-[11px] text-fg-3">
              {answered} of {live.length} answered{answered < live.length ? " · unanswered ones come back next round" : ""}
            </span>
            <ActionButton icon={<Send className="size-3.5" />} pending={sending.busy} pendingLabel="Sending…" onClick={send}>
              Send round
            </ActionButton>
          </div>
        )}
        <ErrorLine error={sending.error && grillError(sending.error)} className="mt-2" />
      </div>
    </div>
  );
}

/** One question of the open round: the recommendation with Accept or Decide for me, and a field for any
 * other answer, saved as a decision or as a comment to discuss. */
type Doing = "accept" | "decide" | "save" | "discuss";

export function QuestionCard({ n, q, agentId, sessionId, round }: { n: number; q: GrillQuestion; agentId: string; sessionId: string; round: number }) {
  const written = (q.state === "answered" && !q.accepted) || q.state === "discuss";
  const saved = q.images ?? [];
  // A half-typed answer, with its pasted images, is this question's Draft, so it survives switching agents or tabs.
  // A saved answer keeps showing its text and thumbnails.
  const [initial] = useState(() => {
    const d = loadAnswerDraft(sessionId, round, q.id);
    return d.text || d.images.length > 0 ? d : { text: written ? (q.answer ?? "") : "", images: written ? saved : [] };
  });
  const [draft, setDraft] = useState(initial.text);
  const [attachments, setAttachments] = useState<Attachment[]>(() => draftImages(agentId, initial.images));
  const saving = useAction();
  // Which button the running save came from: it shows the spinner.
  const [doing, setDoing] = useState<Doing | null>(null);
  const busy = saving.busy;
  const [attachErr, setAttachErr] = useState<string | null>(null);
  // Answers given elsewhere (the app) show here too; on mount the Draft wins.
  const savedList = saved.join("\n");
  const seen = useRef({ written, answer: q.answer, images: savedList });
  useEffect(() => {
    if (seen.current.written === written && seen.current.answer === q.answer && seen.current.images === savedList) return;
    seen.current = { written, answer: q.answer, images: savedList };
    if (!written) return;
    setDraft(q.answer ?? "");
    const files = savedList ? savedList.split("\n") : [];
    // Just saved from here: keep the tiles already shown.
    setAttachments((a) =>
      a.map((x) => x.file).join("\n") === savedList ? a : (a.forEach((x) => URL.revokeObjectURL(x.preview)), draftImages(agentId, files)),
    );
  }, [written, q.answer, savedList, agentId]);

  const addImages = (files: File[]) => setAttachErr(attachImages(agentId, files, attachments.length, setAttachments, "answer"));
  const removeImage = (key: string) =>
    setAttachments((a) => a.filter((x) => (x.key === key ? (URL.revokeObjectURL(x.preview), false) : true)));
  // Images are dropped on the card, not on the chat around it.
  const drop = useFileDrop(addImages, true);
  const uploading = attachments.some((a) => !a.file && !a.error);
  const failed = attachments.some((a) => a.error);
  const images = attachments.flatMap((a) => (a.file ? [a.file] : []));
  const imageList = images.join("\n");

  const run = (what: Doing, f: () => Promise<unknown>) => {
    if (busy) return;
    setDoing(what);
    setAttachErr(null);
    return saving.run(f);
  };
  const pendingOn = (what: Doing) => busy && doing === what;
  // An answer is text, images or both.
  const answer = (text: string, discuss = false, attached: string[] = images, what: Doing = discuss ? "discuss" : "save") => {
    if ((!text.trim() && attached.length === 0) || (attached === images && (uploading || failed))) return;
    run(what, async () => {
      await api.grillAnswer(agentId, sessionId, q.id, text.trim(), discuss, attached);
      if (text === draft && attached === images) clearAnswerDrafts(sessionId, round, [q.id]);
    });
  };

  const same = written && q.answer === draft.trim() && savedList === imageList;
  useEffect(() => {
    saveAnswerDraft(sessionId, round, q.id, same ? { text: "", images: [] } : { text: draft, images: imageList ? imageList.split("\n") : [] });
  }, [sessionId, round, q.id, same, draft, imageList]);
  const empty = !draft.trim() && images.length === 0;
  // Grows with the answer up to 5 lines, then scrolls.
  const field = useRef<HTMLTextAreaElement>(null);
  useAutoGrow(field, draft, 1, 5);
  // "@" picks a file from the agent's workspace, as in the chat; no agents, an answer goes only to this one.
  const mention = useMention(agentId, draft, setDraft);
  const wait = failed ? "Remove the image that failed to upload" : uploading ? "Waiting for images to upload" : undefined;
  return (
    <div
      {...drop.handlers}
      data-grill-question={questionAnchor(sessionId, round, q.id)}
      className={cx("rounded-xl border bg-ink-900/70 p-3 transition", drop.dragging ? "border-dashed border-accent/60" : "border-line")}
    >
      {q.from_round != null && <div className="mb-1 font-mono text-[10.5px] uppercase tracking-wider text-fg-3">Still open from round {q.from_round}</div>}
      <div className="prose-chat text-[14px]">
        <Markdown remarkPlugins={[remarkGfm]}>{`${n}. ${q.text}`}</Markdown>
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <div className="flex min-w-[14rem] flex-1 items-start gap-1.5 text-[13px] text-fg-2">
          <Sparkles className="mt-0.5 size-3.5 shrink-0 text-accent" />
          <span>
            <span className="text-fg-3">Recommended:</span> {q.recommendation}
          </span>
        </div>
        {q.accepted ? (
          <span className="inline-flex items-center gap-1 font-mono text-[11.5px] text-good">
            <Check className="size-3.5" /> Accepted
          </span>
        ) : (
          <ActionButton variant="secondary" className="shrink-0" icon={<Check className="size-3.5" />} pending={pendingOn("accept")} pendingLabel="Accepting…" disabled={busy} onClick={() => answer("Yes", false, [], "accept")}>
            Accept
          </ActionButton>
        )}
        {q.state === "decided" ? (
          <span className="inline-flex items-center gap-1 font-mono text-[11.5px] text-good">
            <Wand2 className="size-3.5" /> Left to the agent
          </span>
        ) : (
          <ActionButton
            variant="ghost"
            className="shrink-0"
            icon={<Wand2 className="size-3.5" />}
            pending={pendingOn("decide")}
            pendingLabel="Deciding…"
            disabled={busy}
            onClick={() => run("decide", () => api.grillCommand(sessionId, "decide", q.id))}
            title="The agent's recommendation becomes the decision"
          >
            Decide for me
          </ActionButton>
        )}
      </div>
      {attachments.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-2">
          {attachments.map((a) => <AttachmentTile key={a.key} a={a} onRemove={() => removeImage(a.key)} onMissing={() => removeImage(a.key)} />)}
        </div>
      )}
      {mention.menu && <div className="mt-2">{mention.menu}</div>}
      <form
        className="mt-2 flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          answer(draft);
        }}
      >
        <textarea
          ref={field}
          rows={1}
          className="input resize-none py-1.5 leading-5"
          placeholder={q.accepted || q.state === "decided" ? "Or answer differently…" : "Your answer, or a comment to discuss…"}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          // Enter saves, like the chat; Shift+Enter adds a line, and Enter that confirms an IME composition doesn't save.
          onKeyDown={(e) => {
            // While the @ menu is open, Enter picks instead of saving.
            if (mention.onKeyDown(e)) return;
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              answer(draft);
            }
          }}
          onPaste={(e) => {
            const files = pastedImages(e);
            if (files.length > 0) addImages(files);
          }}
          aria-label={`Answer to question ${n}`}
        />
        {same && q.state === "answered" ? (
          <span className="inline-flex shrink-0 items-center gap-1 font-mono text-[11.5px] text-good">
            <Check className="size-3.5" /> Saved
          </span>
        ) : (
          <ActionButton
            variant="secondary"
            className="shrink-0"
            type="submit"
            icon={<CornerDownLeft className="size-3.5" />}
            pending={pendingOn("save")}
            disabled={busy || empty || uploading || failed}
            aria-label="Save answer"
            title={wait ?? "Save as your answer"}
          />
        )}
        {same && q.state === "discuss" ? (
          <span className="inline-flex shrink-0 items-center gap-1 font-mono text-[11.5px] text-accent">
            <MessageCircle className="size-3.5" /> To discuss
          </span>
        ) : (
          <ActionButton
            variant="secondary"
            className="shrink-0"
            type="button"
            icon={<MessageCircle className="size-3.5" />}
            pending={pendingOn("discuss")}
            pendingLabel={<span className="max-md:hidden">Saving…</span>}
            disabled={busy || empty || uploading || failed}
            onClick={() => answer(draft, true)}
            title={wait ?? "Not a decision: the agent explains and asks again next round"}
          >
            <span className="max-md:hidden">Discuss</span>
          </ActionButton>
        )}
      </form>
      <ErrorLine error={attachErr ?? (saving.error && grillError(saving.error))} className="mt-1.5" />
    </div>
  );
}
