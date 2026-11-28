/** Claude Code's multiple-choice question tool. Once the user allows it, Loft asks its questions as a second request
 *  (source `questions`) and sends the picks back as the tool's `answers`, keyed by question text. */
export const ASK_TOOL = "AskUserQuestion";

export type Question = {
  question: string;
  header?: string;
  multiSelect?: boolean;
  options?: { label: string; description?: string }[];
};

/** One question's state on the card: the options picked, and what's typed under "Other". */
export type Pick = { chosen: string[]; other: string };

const str = (v: unknown): v is string => typeof v === "string";

/** The questions, with only string text kept: the input comes from the agent, and anything else would crash the card. */
export function questionsOf(input: Record<string, any> | undefined): Question[] {
  const qs = input?.questions;
  if (!Array.isArray(qs)) return [];
  return qs
    .filter((q) => str(q?.question))
    .map((q) => ({
      question: q.question,
      header: str(q.header) ? q.header : undefined,
      multiSelect: q.multiSelect === true,
      options: (Array.isArray(q.options) ? q.options : [])
        .filter((o: any) => str(o?.label))
        .map((o: any) => ({ label: o.label, description: str(o.description) ? o.description : undefined })),
    }));
}

/** The answer given to a question, read safely: a question named `__proto__` or `constructor` reads nothing inherited. */
export function answerFor(answers: unknown, question: string): string | null {
  if (!answers || typeof answers !== "object" || !Object.hasOwn(answers, question)) return null;
  const a = (answers as Record<string, unknown>)[question];
  return str(a) ? a : null;
}

/** Picking an option: a single-choice question keeps only it (picking it again clears it); multi-choice toggles it. */
export function toggle(p: Pick, label: string, multi: boolean): Pick {
  const has = p.chosen.includes(label);
  if (multi) return { ...p, chosen: has ? p.chosen.filter((l) => l !== label) : [...p.chosen, label] };
  return { other: has ? p.other : "", chosen: has ? [] : [label] };
}

/** "Other" text replaces a single-choice pick and joins a multi-choice one. */
export function typeOther(p: Pick, text: string, multi: boolean): Pick {
  return { other: text, chosen: multi || !text.trim() ? p.chosen : [] };
}

function answerOf(p: Pick | undefined): string {
  if (!p) return "";
  return [...p.chosen, p.other.trim()].filter(Boolean).join(", ");
}

/** The `answers` to send, or null until every question has one. */
export function answersOf(qs: Question[], picks: Record<number, Pick>): Record<string, string> | null {
  const out = new Map<string, string>();
  for (const [i, q] of qs.entries()) {
    const a = answerOf(picks[i]);
    if (!a) return null;
    out.set(q.question, a);
  }
  // fromEntries makes own keys, so a question named `__proto__` stays an answer, not a prototype.
  return Object.fromEntries(out);
}
