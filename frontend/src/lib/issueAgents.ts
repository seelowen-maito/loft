/** The agents an issue can be sent to: those whose project is on the issue's repo, any folder of it, A–Z by name.
 * `projects` maps each project to its repo as the Issues tab lists them, compared ignoring case as GitHub does; a project without one never matches.
 * Null while the agents or the projects are still loading, so the picker says "Loading…", not "No agent". */
export function agentsOnRepo<A extends { name: string; project: string }>(agents: A[] | null, projects: { project: string; repo: string | null }[] | null, repo: string): A[] | null {
  if (!agents || !projects) return null;
  const on = new Set(projects.filter((p) => p.repo?.toLowerCase() === repo.toLowerCase()).map((p) => p.project));
  return agents.filter((a) => on.has(a.project.trim())).sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}
