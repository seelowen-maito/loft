import type { Task } from "./api";

/** The action that attaches an issue (Loft or GitHub) to a chat message: label, button and picker title. */
export const ATTACH = "Attach an issue";

export type Source = "all" | "loft" | "github";
export const SOURCES: { key: Source; label: string }[] = [
  { key: "all", label: "All" },
  { key: "loft", label: "Loft" },
  { key: "github", label: "GitHub" },
];

/** The issues matching a search and an origin. "#42" finds the issue mirroring GitHub #42, and only it;
 * a bare "42" finds it too, along with the issues whose text says 42 (a spec's tickets name it). */
export function filterIssues(tasks: Task[], query: string, source: Source): Task[] {
  const q = query.trim().toLowerCase();
  const number = /^#\d+$/.test(q) ? Number(q.slice(1)) : null;
  const bare = /^\d+$/.test(q) ? Number(q) : null;
  const text = (t: Task) => !q || t.title.toLowerCase().includes(q) || t.description.toLowerCase().includes(q) || t.labels.some((l) => l.toLowerCase().includes(q)) || t.owner.toLowerCase() === q.replace(/^@/, "");
  return tasks.filter(
    (t) =>
      (number !== null ? t.github?.number === number : (bare !== null && t.github?.number === bare) || text(t)) &&
      (source === "all" || (source === "github") === !!t.github),
  );
}

/** "Created by me": Loft issues the user created, and GitHub issues opened by `login`, the GitHub user of
 * Loft's token (which includes what Loft published). Without the login, GitHub issues count by who made them in Loft. */
export function createdByMe(t: Task, login: string | null | undefined): boolean {
  if (t.github && login) return (t.github.author ?? "").toLowerCase() === login.toLowerCase();
  return t.created_by === "user";
}
