/** Where an old `/tasks` or `/tasks/:id` link goes now; the query string rides along. */
export function oldTasksTarget(id: string | undefined, search: string) {
  return { pathname: id ? `/issues/${id}` : "/issues", search };
}
