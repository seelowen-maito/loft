import { api, type Agent, type PermissionRequest, type PullRequest, type Repo, type Task } from "../lib/api";
import { createLiveList, upsert } from "./liveList";

export const agentsList = createLiveList<Agent>(api.agents, (as, e) => {
  if (e.type === "agent_updated") {
    return as.some((a) => a.id === e.agent.id) ? as.map((a) => (a.id === e.agent.id ? { ...a, ...e.agent } : a)) : [...as, e.agent];
  }
  if (e.type === "agent_deleted") return as.filter((a) => a.id !== e.agent_id);
  return as;
});

export const pullRequestsList = createLiveList<PullRequest>(() => api.pullRequests(), (ps, e) => (e.type === "pull_request" ? upsert(ps, e.pr) : ps));

export const permissionsList = createLiveList<PermissionRequest>(() => api.permissions(), (ps, e) => (e.type === "permission" ? upsert(ps, e.request) : ps));

export const tasksList = createLiveList<Task>(api.tasks, (ts, e) => {
  if (e.type === "task") return upsert(ts, e.task);
  if (e.type === "task_deleted") return ts.filter((t) => t.id !== e.task_id);
  return ts;
});

// No events: the projects folder rarely changes; a reconnect or the Repos page's Refresh reads it again.
export const reposList = createLiveList<Repo>(() => api.repos(), (rs) => rs);

/** All agents, kept current. */
export const useAgents = agentsList.use;
/** All pull requests the agents registered, kept current. */
export const usePullRequests = pullRequestsList.use;
/** All permission requests, kept current. */
export const usePermissions = permissionsList.use;
/** All issues (tasks), kept current. */
export const useTasks = tasksList.use;
/** The git repos in the projects folder. */
export const useRepos = reposList.use;
