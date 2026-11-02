/** The verdict labels a checker puts on a PR (tests, QA, security), as chips: green for a pass, red for a failure. */
export type Verdict = { label: string; tone: "good" | "bad" };

const PASS = ["tests:green", "qa:passed", "security:passed"];
const FAIL = ["tests:failing", "qa:failed", "security:findings"];

/** The PR's verdict labels in a fixed order (tests, QA, security); other labels aren't verdicts. */
export function verdicts(labels: string[] | undefined): Verdict[] {
  const have = new Set((labels ?? []).map((l) => l.toLowerCase()));
  return ["tests", "qa", "security"].flatMap((kind): Verdict[] => {
    const bad = FAIL.find((l) => l.startsWith(kind) && have.has(l));
    const good = PASS.find((l) => l.startsWith(kind) && have.has(l));
    return bad ? [{ label: bad, tone: "bad" }] : good ? [{ label: good, tone: "good" }] : [];
  });
}
