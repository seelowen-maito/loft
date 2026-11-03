/** The end card's team check as a suggestion, from its `missing_roles`. A missing role never stops a Delivery
 *  (the planning agent builds when there is no Dev, and a missing reviewer is skipped), so say that, then which
 *  agents would help. */
export function teamNote(missing: string[]): string {
  const has = (r: string) => missing.includes(r);
  const reviewers = [has("qa") && "a QA", has("security") && "a Security"].filter(Boolean);
  const parts = [
    has("dev") && "a Dev to build them",
    reviewers.length > 0 && `${reviewers.join(" and ")} agent to have each ticket reviewed`,
    has("code_reviewer") && "a Code reviewer for the spec PR",
  ].filter((p): p is string => !!p);
  const works = has("dev") ? "Delivery works with this agent building the tickets." : "Delivery works with this team.";
  if (parts.length === 0) return "";
  const last = parts.pop();
  return `${works} Add ${parts.length ? `${parts.join(", ")}, and ${last}` : last}.`;
}
