/** The agents an issue can be sent to: those whose project is on the issue's repo, any folder of it, A–Z by name.
 * `projects` maps each project to its repo as the Issues tab lists them; a project without one never matches. */
export function agentsOnRepo<A extends { name: string; project: string }>(agents: A[], projects: { project: string; repo: string | null }[], repo: string): A[] {
  const on = new Set(projects.filter((p) => p.repo === repo).map((p) => p.project));
  return agents.filter((a) => on.has(a.project.trim())).sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}
