/** Which issues Loft runs as a Delivery of their own. */
type Row = { state: string; github?: { state: string } | null; labels: string[]; parent_id: string | null; description: string };

const openOnGithub = (t: { state: string; github?: { state: string } | null }) => t.state === "open" && t.github?.state === "open";

/** A standalone ticket on GitHub (open, `ready-for-agent`, under no spec): Loft runs it as a Delivery.
 *  A spec's ticket has a parent, or names its spec in the ticket template's `## Parent` section. */
export function isStandaloneTicket(t: Row) {
  return openOnGithub(t) && t.labels.includes("ready-for-agent") && !t.parent_id && !/^## Parent\s+#\d+/m.test(t.description);
}

/** An open spec on GitHub (labelled `spec`): Loft delivers its tickets, its open sub-issues, from its page. */
export function isDeliverableSpec(t: Omit<Row, "parent_id" | "description">) {
  return openOnGithub(t) && t.labels.some((l) => l.toLowerCase() === "spec");
}
