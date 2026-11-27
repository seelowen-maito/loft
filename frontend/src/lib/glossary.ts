/** A GLOSSARY.md term: its section, its name (with a qualifier such as "(ticket)"), its meaning and the words to avoid. */
export type Term = { section: string; term: string; meaning: string; avoid: string };

/** The terms of GLOSSARY.md, in order: each `**Term**:` line, the lines under it, and its `_Avoid_:` line. */
export function parseGlossary(md: string): Term[] {
  const terms: Term[] = [];
  let section = "";
  let cur: Term | null = null;
  for (const raw of md.split("\n")) {
    const line = raw.trim();
    const head = /^##\s+(.+)$/.exec(line);
    const start = /^\*\*(.+?)\*\*(.*?):\s*(.*)$/.exec(line);
    if (head) {
      section = head[1];
      cur = null;
    } else if (start) {
      cur = { section, term: (start[1] + start[2]).trim(), meaning: start[3], avoid: "" };
      terms.push(cur);
    } else if (!line) {
      cur = null;
    } else if (cur) {
      const avoid = /^_Avoid_:\s*(.*)$/.exec(line);
      if (avoid) cur.avoid = avoid[1];
      else cur.meaning = cur.meaning ? `${cur.meaning} ${line}` : line;
    }
  }
  return terms;
}
