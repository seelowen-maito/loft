/** Drafts: what the user typed but hasn't sent, kept per browser so switching agents, tabs or reloading never loses it.
 * The composer's is per agent; a grill answer's is per question of a round. `storage` defaults to localStorage,
 * read inside the try since it can throw when site data is blocked. */

import { replyOf, type Quote } from "./reply.ts";

type Store = Pick<Storage, "getItem" | "setItem" | "removeItem">;

/** The composer's text, its uploaded images (only those that finished uploading), its attached task and the
 * message it replies to. */
export type Draft = { text: string; images: string[]; task: string | null; reply: Quote | null };

export const EMPTY_DRAFT: Draft = { text: "", images: [], task: null, reply: null };

/** A grill answer's Draft: its text and uploaded images. */
export type AnswerDraft = { text: string; images: string[] };

const agentKey = (agent: string) => `loft:draft:${agent}`;
const answerKey = (session: string, round: number, question: string) => `loft:draft:grill:${session}:${round}:${question}`;

function read(key: string, storage?: Store): string | null {
  try {
    return (storage ?? localStorage).getItem(key);
  } catch {
    return null;
  }
}

/** Writes `value`, or removes the key when it's null so empty drafts don't pile up. */
function write(key: string, value: string | null, storage?: Store) {
  try {
    const s = storage ?? localStorage;
    if (value === null) s.removeItem(key);
    else s.setItem(key, value);
  } catch {
    /* storage unavailable or full */
  }
}

const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((f): f is string => typeof f === "string") : []);

export function loadDraft(agent: string, storage?: Store): Draft {
  try {
    const d = JSON.parse(read(agentKey(agent), storage) ?? "null");
    return { text: typeof d?.text === "string" ? d.text : "", images: strings(d?.images), task: typeof d?.task === "string" ? d.task : null, reply: replyOf({ reply_to: d?.reply }) };
  } catch {
    return EMPTY_DRAFT;
  }
}

export function saveDraft(agent: string, d: Draft, storage?: Store) {
  const empty = !d.text && d.images.length === 0 && !d.task && !d.reply;
  write(agentKey(agent), empty ? null : JSON.stringify(d), storage);
}

/** Sent on `window` with an agent id after its Draft was changed outside its composer, which then shows it. */
export const DRAFT_SET = "loft:draft-set";

export function loadAnswerDraft(session: string, round: number, question: string, storage?: Store): AnswerDraft {
  try {
    const d = JSON.parse(read(answerKey(session, round, question), storage) ?? "null");
    return { text: typeof d?.text === "string" ? d.text : "", images: strings(d?.images) };
  } catch {
    return { text: "", images: [] };
  }
}

export function saveAnswerDraft(session: string, round: number, question: string, d: AnswerDraft, storage?: Store) {
  const empty = !d.text && d.images.length === 0;
  write(answerKey(session, round, question), empty ? null : JSON.stringify(d), storage);
}

/** The Round note's Draft is kept as one more answer draft of its round, under this key (question ids are UUIDs). */
export const NOTE_DRAFT = "note";

/** Deletes the answer drafts of these questions: one saved, or every question of a round that closed. */
export function clearAnswerDrafts(session: string, round: number, questions: string[], storage?: Store) {
  for (const q of questions) write(answerKey(session, round, q), null, storage);
}
