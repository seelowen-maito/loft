/** Claude Code's multiple-choice question tool. Loft shows its questions in place of a permission prompt and sends
 *  the picks back as the tool's `answers`, keyed by question text. */
export const ASK_TOOL = "AskUserQuestion";

export type Question = {
  question: string;
  header?: string;
  multiSelect?: boolean;
  options?: { label: string; description?: string }[];
};

/** One question's state on the card: the options picked, and what's typed under "Other". */
export type Pick = { chosen: string[]; other: string };

export function questionsOf(input: Record<string, any> | undefined): Question[] {
  const qs = input?.questions;
  return Array.isArray(qs) ? qs.filter((q): q is Question => typeof q?.question === "string") : [];
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
  const out: Record<string, string> = {};
  for (const [i, q] of qs.entries()) {
    const a = answerOf(picks[i]);
    if (!a) return null;
    out[q.question] = a;
  }
  return out;
}
