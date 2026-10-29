import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api } from "./api";
import { loadDraft, saveDraft, type Draft } from "./draft";

/** A pasted image: shown at once from `preview`, sendable once uploaded (`file`). */
export type Attachment = { key: string; preview: string; file?: string; error?: string };

/** A Draft's images, already uploaded: they preview from the server. */
const draftImages = (agent: string, files: string[]): Attachment[] => files.map((file) => ({ key: file, preview: api.uploadUrl(agent, file), file }));

/** The composer's state, kept as the shown agent's Draft: loaded when the agent is shown, saved on every change. */
export function useDraft(agent: string) {
  const [initial] = useState(() => loadDraft(agent));
  const [text, setText] = useState(initial.text);
  const [attachments, setAttachments] = useState<Attachment[]>(() => draftImages(agent, initial.images));
  const [taskId, setTaskId] = useState<string | null>(initial.task);
  const showDraft = (id: string, d: Draft) => {
    setText(d.text);
    setTaskId(d.task);
    setAttachments((a) => (a.forEach((x) => URL.revokeObjectURL(x.preview)), draftImages(id, d.images)));
  };
  // On a switch the new agent's Draft is swapped in before anything renders or saves,
  // so one agent's text never lands in another's. Uploads still running for the old agent are dropped.
  const [draftFor, setDraftFor] = useState(agent);
  if (draftFor !== agent) {
    setDraftFor(agent);
    showDraft(agent, loadDraft(agent));
  }
  const shown = useRef(agent);
  useLayoutEffect(() => {
    shown.current = agent;
  }, [agent]);
  // Commands finish after an await, maybe with another agent on screen: they change the Draft of the agent they ran for.
  const putDraft = (id: string, d: Draft) => {
    saveDraft(id, d);
    if (shown.current === id) showDraft(id, d);
  };

  const images = attachments.flatMap((a) => (a.file ? [a.file] : []));
  // Every change is saved as this agent's Draft; an emptied composer deletes it.
  const imageList = images.join("\n");
  useEffect(() => {
    saveDraft(agent, { text, images: imageList ? imageList.split("\n") : [], task: taskId });
  }, [agent, text, imageList, taskId]);

  // Also drops a Draft's image whose file is gone (the chat history was cleared): it can't be shown or sent.
  const removeImage = (key: string) =>
    setAttachments((a) => a.filter((x) => (x.key === key ? (URL.revokeObjectURL(x.preview), false) : true)));

  return { text, setText, attachments, setAttachments, images, taskId, setTaskId, putDraft, removeImage };
}
