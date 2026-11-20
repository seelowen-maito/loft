/** The answer card above the composer when the agent's latest message asks you something. The asks and either/or
 * rules are the iPhone's (`ChatReplies.asksSomething` / `offersAChoice`), so both apps offer a card for the same messages. */

/** The text outside ``` code blocks. */
const prose = (text: string) => text.split("```").filter((_, i) => i % 2 === 0).join("\n");

const paragraphs = (text: string) => prose(text).split("\n\n").map((p) => p.trim()).filter(Boolean);

/** The message ends on a question: its last paragraph, outside code, ends with a question mark ("Why did it fail? A
 * missing key." answers its own question). */
export function asksSomething(text: string): boolean {
  const last = paragraphs(text).at(-1);
  return !!last && last.replace(/[ *_)"”']+$/, "").endsWith("?");
}

/** Lower case, without accents or punctuation, words split by single spaces: "Faço o merge?" → "faco o merge". */
export const normalize = (s: string) =>
  s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase().replace(/’/g, "'").replace(/[^\p{L}\p{N}']+/gu, " ").trim();

/** The question offers a choice ("merge now or wait for QA?"), so a bare "yes" or "no" means nothing. */
export function offersAChoice(text: string): boolean {
  const n = ` ${normalize(text)} `;
  return n.includes(" or ") || n.includes(" ou ");
}

const LIST_ITEM = /^(?:\d+[.)]|[-*•])\s+(.+)$/;

/** The question the card quotes: the last paragraph, without a list it starts with ("1. Merge\n2. Wait\nWhich?"). */
export function question(text: string): string {
  const lines = (paragraphs(text).at(-1) ?? "").split("\n");
  const start = lines.findLastIndex((l) => LIST_ITEM.test(l.trim())) + 1;
  return lines.slice(start).map((l) => l.trim()).filter(Boolean).join(" ");
}

/** The numbered or bulleted list right before the question, one option per top-level item ("**Merge now**" → "Merge now"). */
export function listedOptions(text: string): string[] {
  const lines = prose(text).trimEnd().split("\n");
  let i = lines.length - 1;
  // The question's own lines.
  while (i >= 0 && lines[i].trim() && !LIST_ITEM.test(lines[i].trim())) i--;
  const items: string[] = [];
  for (; i >= 0; i--) {
    const line = lines[i];
    if (!line.trim()) continue;
    // A nested item or an item's second line belongs to the item above it.
    if (/^\s{2,}/.test(line)) continue;
    const m = LIST_ITEM.exec(line.trim());
    if (!m) break;
    items.unshift(m[1].replace(/\*\*|__|`/g, "").replace(/[\s,;.]+$/, "").replace(/\s+or$/i, "").trim());
  }
  return items.filter(Boolean);
}

/** Words a question asking for something other than yes or no starts with ("Which port?", "Qual porta?"). */
const OPEN_WORDS = new Set(["what", "which", "where", "when", "why", "how", "who", "whom", "whose", "qual", "quais", "quando", "onde", "porque", "como", "quem", "quanto", "quantos", "quantas", "que"]);

/** The question's last sentence starts like a yes/no question: not "what…", "which…", nor an either/or. */
export function yesNo(q: string): boolean {
  if (offersAChoice(q)) return false;
  const sentence = q.split(/(?<=[.!?])\s+/).filter(Boolean).at(-1) ?? "";
  const first = normalize(sentence).split(" ")[0] ?? "";
  return !!first && !OPEN_WORDS.has(first) && !normalize(sentence).startsWith("o que") && !normalize(sentence).startsWith("por que");
}

type Msg = { id: number; role: string; kind: string; content: string };

export type AnswerCard<M extends Msg = Msg> = { message: M; question: string; answers: string[] };

/** At most this many option buttons; a longer list is answered with Reply…. */
const MAX_OPTIONS = 6;

/** The card for the agent's latest chat message when it asks something; null while a permission, an AskUserQuestion
 * or a grill waits on the user (they have their own cards), or when the user had the last word. */
export function answerCard<M extends Msg>(messages: M[], waitingOnYou: boolean): AnswerCard<M> | null {
  if (waitingOnYou) return null;
  const m = messages.findLast((x) => x.kind === "text" || x.role === "user");
  if (!m || m.role !== "assistant" || m.kind !== "text" || !asksSomething(m.content)) return null;
  const q = question(m.content);
  const options = listedOptions(m.content);
  const answers = offersAChoice(q) ? (options.length >= 2 && options.length <= MAX_OPTIONS ? options : []) : yesNo(q) ? ["Yes", "No"] : [];
  return { message: m, question: q, answers };
}
