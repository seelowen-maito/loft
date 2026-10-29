import type { Task } from "./api";

export type Source = "all" | "loft" | "github";
export const SOURCES: { key: Source; label: string }[] = [
  { key: "all", label: "All" },
  { key: "loft", label: "Loft" },
  { key: "github", label: "GitHub" },
];

/** The issues matching a search and an origin. "#42" finds the issue mirroring GitHub #42, and only it. */
export function filterIssues(tasks: Task[], query: string, source: Source): Task[] {
  const q = query.trim().toLowerCase();
  const number = /^#\d+$/.test(q) ? Number(q.slice(1)) : null;
  return tasks.filter(
    (t) =>
      (number !== null ? t.github?.number === number : !q || t.title.toLowerCase().includes(q) || t.description.toLowerCase().includes(q) || t.labels.some((l) => l.toLowerCase().includes(q)) || t.owner.toLowerCase() === q.replace(/^@/, "")) &&
      (source === "all" || (source === "github") === !!t.github),
  );
}
