/** Which issues Loft runs as a Delivery of their own. */
type Row = { state: string; github?: { state: string } | null; labels: string[]; parent_id: string | null; description: string };

/** A standalone ticket on GitHub (open, `ready-for-agent`, under no spec): Loft runs it as a Delivery.
 *  A spec's ticket has a parent, or names its spec in the ticket template's `## Parent` section. */
export function isStandaloneTicket(t: Row) {
  return t.state === "open" && t.github?.state === "open" && t.labels.includes("ready-for-agent") && !t.parent_id && !/^## Parent\s+#\d+/m.test(t.description);
}
