/** Drafts: what the user typed but hasn't sent, kept per browser so switching agents, tabs or reloading never loses it.
 * The composer's is per agent; a grill answer's is per question of a round. `storage` defaults to localStorage,
 * read inside the try since it can throw when site data is blocked. */

type Store = Pick<Storage, "getItem" | "setItem" | "removeItem">;

/** The composer's text, its uploaded images (only those that finished uploading) and its attached task. */
export type Draft = { text: string; images: string[]; task: string | null };

export const EMPTY_DRAFT: Draft = { text: "", images: [], task: null };

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
    return { text: typeof d?.text === "string" ? d.text : "", images: strings(d?.images), task: typeof d?.task === "string" ? d.task : null };
  } catch {
    return EMPTY_DRAFT;
  }
}

export function saveDraft(agent: string, d: Draft, storage?: Store) {
  const empty = !d.text && d.images.length === 0 && !d.task;
  write(agentKey(agent), empty ? null : JSON.stringify(d), storage);
}

export function clearDraft(agent: string, storage?: Store) {
  write(agentKey(agent), null, storage);
}

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

/** Deletes the answer drafts of these questions: one saved, or every question of a round that closed. */
export function clearAnswerDrafts(session: string, round: number, questions: string[], storage?: Store) {
  for (const q of questions) write(answerKey(session, round, q), null, storage);
}
