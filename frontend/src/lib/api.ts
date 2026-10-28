export type Avatar = { svg: string };

/** A weekly window with no autonomous wake-ups, in the Mac's local time. Days: 0 = Monday … 6 = Sunday. */
export type QuietWindow = { days: number[]; start: string; end: string };

export type Agent = {
  id: string;
  name: string;
  title: string;
  /** Free-form project or tag; agents sharing one are grouped. */
  project: string;
  personality: string;
  prompt: string;
  model: string;
  effort: string;
  interval_minutes: number;
  quiet_hours: QuietWindow[];
  allowed_tools: string[];
  disallowed_tools: string[];
  mcp_servers: Record<string, unknown>;
  permission_mode: string;
  /** Model for every subagent it spawns; "" is Claude Code's default. */
  subagent_model: string;
  /** Subagents running at once; null is Claude Code's default, 0 turns them off. */
  max_subagents: number | null;
  /** Leads its project's team (one per project). Wears a captain hat. */
  lead?: boolean;
  /** Its part in the end-of-planning team check: "dev", "reviewer" or "" (neither). */
  role?: string;
  avatar: Avatar;
  status: "idle" | "running" | "waiting" | "error" | string;
  status_text: string;
  session_id: string | null;
  /** Tokens in the Claude session's context as of its last reply; null until a run reports it. */
  context_tokens?: number | null;
  last_run_at: string | null;
  next_run_at: string | null;
  total_cost_usd: number;
  created_at: string;
  /** Loft's own agent: always there, and it can't be deleted. */
  builtin?: boolean;
  /** The agent that created this one, if an agent did. */
  created_by?: string | null;
  running?: boolean;
  workspace?: string;
  /** Last event from claude in the current run; null when not running. */
  last_output_at?: string | null;
  /** Background tasks the agent waits on after it has replied, and since when. */
  waiting_on?: string[];
  waiting_since?: string | null;
};

export type Message = {
  id: number;
  agent_id: string;
  run_id: string | null;
  role: "user" | "assistant" | "system" | "tool";
  kind: "text" | "thinking" | "tool_use" | "tool_result" | "permission" | "error" | "notice" | "run_start" | "run_end" | "clear" | "compact" | "resume";
  content: string;
  meta: Record<string, any>;
  created_at: string;
};

/** One of an agent's past Sessions, from Claude Code's transcripts (`/api/agents/{id}/sessions`). */
export type PastSession = {
  id: string;
  started_at: string;
  first_message: string;
  context_tokens: number | null;
};

export type TaskStatus = "inbox" | "ready" | "in_progress" | "blocked" | "waiting_approval" | "done" | "failed" | "cancelled";

/** A task on the shared list (`/api/tasks`). */
export type Task = {
  id: string;
  title: string;
  description: string;
  /** todo (personal) | work (meant for an agent) */
  kind: string;
  status: TaskStatus;
  priority: "low" | "normal" | "high" | "urgent";
  project: string;
  agent_id: string | null;
  parent_id: string | null;
  depends_on: string[];
  due_at: string | null;
  /** "user", or the id of the agent that created it */
  created_by: string;
  note: string;
  summary: string;
  error: string;
  created_at: string;
  updated_at: string;
  started_at: string | null;
  completed_at: string | null;
};

/** One entry in a task's thread. */
export type TaskComment = {
  id: number;
  task_id: string;
  /** "user", or an agent id */
  author: string;
  kind: "comment" | "note" | "status";
  text: string;
  meta: { author_name?: string; status?: string; summary?: string; error?: string; private?: boolean; to_agent?: string; client_id?: string };
  created_at: string;
};

/** A task split into subtasks (`GET /api/tasks/{id}/plan`). */
export type TaskPlan = {
  task: { id: string; title: string; status: TaskStatus; agent: { id: string; name: string } | null; summary: string };
  subtasks: {
    id: string;
    title: string;
    status: TaskStatus;
    agent: { id: string; name: string } | null;
    note: string;
    summary: string;
    error: string;
    updated_at: string;
    prs: { id: string; slug: string; number: number; title: string; url: string; state: string }[];
  }[];
  progress: { total: number; done: number; in_progress: number; waiting: number; failed: number; todo: number; cancelled: number; percent: number };
};

/** One question of a grill round, with the user's answer so far. */
export type GrillQuestion = {
  id: string;
  text: string;
  speak: string;
  kind: string;
  options: string[];
  /** The agent's proposed answer; "yes" accepts it. */
  recommendation: string;
  /** discuss: the answer is a comment, not a decision; decided: left to the agent, its recommendation stands; carried: sent unanswered. */
  state: "open" | "answered" | "discuss" | "decided" | "carried";
  answer: string | null;
  /** The answer is the recommendation, accepted as-is. */
  accepted: boolean;
  /** Carried over unanswered: the round it was first asked in. */
  from_round: number | null;
};

/** An interview (grill) the agent is running, in rounds; `GET /agents/{id}/grill`. */
export type GrillSession = {
  session_id: string;
  agent_id: string;
  state: "preparing" | "active" | "paused" | "ended";
  end_reason: string | null;
  topic: string;
  /** The latest round's number, its size and how many of its questions have an answer. */
  round: number | null;
  total: number | null;
  answered: number | null;
  /** The latest round was sent; the agent is working on the next one. */
  waiting: boolean;
  questions: GrillQuestion[] | null;
  started_at: string;
  updated_at: string;
};

/** A link to an issue the planning published. */
export type PlannedIssue = { number: number; title: string; url: string };

/** The end-of-planning card on a grill's final summary (`meta.grill.end_card`). */
export type EndCard = {
  repo: string;
  spec: PlannedIssue | null;
  tickets: PlannedIssue[];
  /** Left in the composer after Clear context, never sent by Loft. */
  next_message: string;
  /** Roles no agent in the project fills: "dev", "reviewer". */
  missing_roles: string[];
  /** The message "Set up the team with lofty" sends, when roles are missing; `sent_at` once it was sent. */
  setup: { agent_id: string; message: string; sent_at?: string } | null;
};

/** A phone registered for push (`/api/devices`). */
export type PushDevice = {
  token: string;
  platform: string;
  environment: "development" | "production";
  bundle_id: string;
  name: string;
  kinds: string[];
  quiet_hours: { start: string; end: string; tz: string; utc_offset_minutes: number } | null;
  created_at: string;
  last_seen_at: string;
};

/** `GET /api/push`: whether APNs is set up. The key itself is never returned. */
export type PushStatus = { configured: boolean; key_present: boolean; key_id: string; team_id: string; bundle_id: string; devices: number };

export type Memory = { id: number; agent_id: string; title: string; content: string; tags: string; created_at: string };

export type PermissionRequest = {
  id: string;
  agent_id: string;
  source: "prompt" | "request";
  tool_name: string;
  input: Record<string, any>;
  reason: string;
  rule: string;
  status: "pending" | "allowed_once" | "allowed_always" | "denied" | "expired";
  created_at: string;
};

export type Commit = {
  hash: string;
  subject: string;
  author: string;
  timestamp: number;
  insertions: number | null;
  deletions: number | null;
};

export type Repo = {
  name: string;
  path: string;
  branch: string;
  remote: string | null;
  github: string | null;
  ahead: number;
  behind: number;
  staged: number;
  modified: number;
  untracked: number;
  stashes: number;
  branches: number;
  total_commits: number;
  last_commit: Commit | null;
  daily: number[];
  languages: [string, number][];
  contributors: [string, number][];
  /** the project's websites, from `sites.json` at the repo root */
  sites: Site[];
  error: string | null;
};

export type Site = {
  name: string;
  url: string;
  description: string | null;
  group: string | null;
  /** only reachable from this machine or the tailnet */
  private: boolean;
};

export type GithubItem = {
  number: number;
  title: string;
  author: string;
  url: string;
  draft: boolean;
  labels: string[];
  comments: number;
  created_at: string;
  updated_at: string;
};

export type GithubInfo = {
  slug: string;
  stars: number;
  forks: number;
  watchers: number;
  default_branch: string;
  description: string | null;
  pulls: GithubItem[];
  issues: GithubItem[];
  error: string | null;
};

export type PullRequest = {
  id: string;
  agent_id: string;
  slug: string;
  number: number;
  url: string;
  tool_use_id: string | null;
  title: string;
  state: "open" | "closed" | "merged" | "";
  draft: boolean;
  mergeable: boolean | null;
  mergeable_state: string;
  merge_methods: string[];
  head_ref: string;
  base_ref: string;
  author: string;
  additions: number;
  deletions: number;
  changed_files: number;
  commits: number;
  comments: number;
  checks: { total: number; passed: number; failed: number; pending: number } | null;
  merged_by: string | null;
  merged_at: string | null;
  closed_at: string | null;
  error: string | null;
  checked_at: string | null;
  created_at: string;
};

export type AgentSummary = {
  agent: Agent;
  days: number;
  /** Runs per local day, oldest first, ending today. */
  daily_auto: number[];
  daily_user: number[];
  tool_calls: number;
  /** Set when the agent's latest message was an error. */
  last_error: string | null;
  cost_usd: number;
  user_messages: number;
  top_tools: [string, number][];
  last_reply: { text: string; at: string } | null;
  memories: number;
  pending_permissions: number;
  prs: { total: number; open: number; merged: number };
};

export type FileEntry = { path: string; dir: boolean; depth: number; size: number; is_repo: boolean };

export type UpdateStatus = {
  phase: "disabled" | "up_to_date" | "waiting" | "updating" | "error";
  current: string;
  latest: string | null;
  running_agents: number;
  error: string | null;
};

export type Settings = {
  projects_dir: string;
  claude_bin: string;
  ssh_host: string;
  /** Always "" from the server; see SettingsView.github_token_set. */
  github_token: string;
  auto_update: boolean;
  show_simulator: boolean;
};

export type SettingsView = {
  settings: Settings;
  github_token_set: boolean;
  /** Fields that only apply after a restart. */
  restart_fields: string[];
  /** Restart-only fields saved but not yet in effect. */
  pending_restart: string[];
  running_agents: number;
};

/** Claude plan limits, as in Claude Code's /usage. percent is 0–100. */
export type UsageWindow = { key: string; label: string; percent: number; resets_at: string | null };
export type Usage = { windows: UsageWindow[]; error: string | null };

/** The Mac's hottest CPU die sensor and memory, sampled every 30 s. Null where it can't be read. */
export type SystemReading = {
  cpu_temp_c: number | null;
  level: "nominal" | "warm" | "hot" | null;
  /** Active + wired + compressed, like Activity Monitor's "Memory Used". */
  mem_used_bytes: number | null;
  mem_total_bytes: number | null;
  /** high at ≥ 80 % used, critical at ≥ 90 %. */
  mem_level: "nominal" | "high" | "critical" | null;
  sampled_at: string | null;
};

export type Info = { projects_dir: string; storage_dir: string; github_token: boolean; version: string };

/** A GitHub issue as the Issues tab lists it. */
export type Issue = {
  number: number;
  title: string;
  state: "open" | "closed";
  state_reason: string | null;
  url: string;
  author: string;
  labels: { name: string; color: string; description: string | null }[];
  assignees: { login: string; avatar_url: string }[];
  comments: number;
  created_at: string;
  updated_at: string;
  sub_issues: { total: number; completed: number };
  blocked_by: number;
  blocking: number;
};

export type IssueLink = { number: number; title: string; state: "open" | "closed"; url: string };

/** One issue with its body, a spec's sub-issues and a ticket's dependencies. */
export type IssueDetail = Issue & {
  project: string;
  repo: string;
  body: string;
  sub_issue_list: IssueLink[];
  blocked_by_list: IssueLink[];
  blocking_list: IssueLink[];
};

/** A project's issues. `problem`: it has no GitHub repo; `error`: GitHub turned the request down; `truncated`: more than the pages Loft reads. */
export type ProjectIssues = { project: string; repo: string | null; problem?: string | null; error?: string; truncated?: boolean; issues: Issue[] };

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body !== undefined ? { "content-type": "application/json" } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error((data as any).error ?? `${res.status} ${res.statusText}`);
  return data as T;
}

/** A grill request's error as text for the user: the 409 codes the server sends when the round was sent or the
 * interview ended elsewhere. Same list as the app's `LoftStore.grillFailure`. */
export function grillError(e: unknown) {
  const m = (e as Error).message;
  if (m === "grill_closed") return "This interview has already ended.";
  if (m === "round_closed" || m === "question_closed") return "This round was already sent.";
  return m;
}

export type Simulator = { udid: string; name: string; runtime: string };
/** Points are fractions of the screen, 0–1. */
export type SimulatorInput =
  | { action: "tap"; x: number; y: number }
  | { action: "swipe"; x: number; y: number; x2: number; y2: number; duration: number }
  | { action: "type"; text: string }
  | { action: "key"; code: number }
  | { action: "button"; name: "home" | "lock" };

export const api = {
  simulators: () => req<{ interactive: boolean; devices: Simulator[] }>("GET", "/simulator"),
  simulatorInput: (udid: string, input: SimulatorInput) => req<{ ok: boolean }>("POST", "/simulator/input", { udid, ...input }),
  info: () => req<Info>("GET", "/info"),
  update: () => req<UpdateStatus>("GET", "/update"),
  usage: () => req<Usage>("GET", "/usage"),
  system: () => req<SystemReading>("GET", "/system"),
  settings: () => req<SettingsView>("GET", "/settings"),
  /** github_token: undefined keeps the saved token, "" removes it. */
  saveSettings: (s: Omit<Settings, "github_token"> & { github_token?: string }) => req<SettingsView>("PUT", "/settings", s),
  restart: () => req<{ ok: boolean }>("POST", "/restart"),
  repos: (refresh = false) => req<Repo[]>("GET", `/repos${refresh ? "?refresh=true" : ""}`),
  github: (refresh = false) => req<Record<string, GithubInfo>>("GET", `/repos/github${refresh ? "?refresh=true" : ""}`),
  /** Every project's GitHub issues (or just `project`'s), fetched live; nothing is fetched when `connected` is false. */
  issues: (project?: string) => req<{ connected: boolean; projects: ProjectIssues[] }>("GET", project ? `/issues?project=${encodeURIComponent(project)}` : "/issues"),
  issue: (project: string, n: number) => req<IssueDetail>("GET", `/issues/${n}?project=${encodeURIComponent(project)}`),
  /** Sends `agent` a chat message with the issue's number and link, which wakes it; it must be on the issue's repo. */
  sendIssue: (project: string, n: number, agent: string) => req<{ ok: boolean; queued: boolean }>("POST", `/issues/${n}/send`, { project, agent }),
  commits: (name: string) => req<Commit[]>("GET", `/repos/${encodeURIComponent(name)}/commits`),

  agents: () => req<Agent[]>("GET", "/agents"),
  agentsSummary: () => req<AgentSummary[]>("GET", "/agents/summary"),
  createAgent: (a: Partial<Agent>) => req<Agent>("POST", "/agents", a),
  reorderAgents: (ids: string[]) => req<{ ok: boolean }>("PUT", "/agents/order", { ids }),
  updateAgent: (id: string, a: Partial<Agent>) => req<Agent & { notice?: string }>("PATCH", `/agents/${id}`, a),
  deleteAgent: (id: string, purge: boolean) => req<{ ok: boolean }>("DELETE", `/agents/${id}?purge_files=${purge}`),
  messages: (id: string) => req<Message[]>("GET", `/agents/${id}/messages?limit=400`),
  /** `taskId` attaches a task: the agent gets its title, status and latest note with the message. */
  send: (id: string, text: string, images: string[] = [], taskId?: string) =>
    req<{ ok: boolean; queued: boolean }>("POST", `/agents/${id}/messages`, { text, images, ...(taskId ? { meta: { task_id: taskId } } : {}) }),
  /** "Set up the team with lofty" on an end-of-planning card: sends its setup message once. */
  sendTeamSetup: (id: string, messageId: number) =>
    req<{ ok: boolean; duplicate?: boolean; message: Message }>("POST", `/agents/${id}/messages/${messageId}/team-setup`),
  /** Stores a pasted image for the next message; send its `file` in `send`'s images. */
  upload: async (id: string, image: Blob) => {
    const res = await fetch(`/api/agents/${id}/uploads`, { method: "POST", body: image });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.error ?? `${res.status} ${res.statusText}`);
    return data as { file: string };
  },
  uploadUrl: (id: string, file: string) => `/api/agents/${id}/uploads/${file}`,
  clearMessages: (id: string) => req("DELETE", `/agents/${id}/messages`),
  run: (id: string) => req("POST", `/agents/${id}/run`),
  stop: (id: string) => req("POST", `/agents/${id}/stop`),
  /** Stops a run that waits or went quiet; queued messages are kept and run next. */
  tasks: () => req<Task[]>("GET", "/tasks"),
  task: (id: string) => req<Task>("GET", `/tasks/${id}`),
  pushStatus: () => req<PushStatus>("GET", "/push"),
  configurePush: (b: { key_id?: string; team_id?: string; bundle_id?: string; key_p8?: string; remove_key?: boolean }) => req<PushStatus>("PUT", "/push", b),
  devices: () => req<PushDevice[]>("GET", "/devices"),
  testPush: (token: string) => req<{ ok: boolean }>("POST", `/devices/${token}/test`),
  removeDevice: (token: string) => req<{ ok: boolean }>("DELETE", `/devices/${token}`),
  taskPlan: (id: string) => req<TaskPlan>("GET", `/tasks/${id}/plan`),
  taskComments: (id: string) => req<TaskComment[]>("GET", `/tasks/${id}/comments?limit=200`),
  /** `send`: deliver to the task's agent (else a private note); `reopen`: run a finished task again with it. */
  commentTask: (id: string, text: string, opts: { send: boolean; reopen: boolean }) =>
    req<{ comment: TaskComment | null; queued?: boolean; delivered_to?: string | null; task?: Task }>("POST", `/tasks/${id}/comments`, {
      text,
      client_id: crypto.randomUUID(),
      ...opts,
    }),
  createTask: (t: Partial<Task> & { title: string }) => req<Task>("POST", "/tasks", { ...t, id: crypto.randomUUID() }),
  updateTask: (id: string, p: Partial<Task>) => req<Task>("PATCH", `/tasks/${id}`, p),
  runTask: (id: string, agentId?: string, note = "") => req<{ task: Task; queued: boolean }>("POST", `/tasks/${id}/execute`, { agent_id: agentId, note }),
  cancelTask: (id: string, stopRun: boolean) => req<{ task: Task; stopped: boolean }>("POST", `/tasks/${id}/cancel`, { stop_run: stopRun }),
  deleteTask: (id: string) => req<{ ok: boolean }>("DELETE", `/tasks/${id}`),
  /** The agent's open interview, or null. */
  grill: (id: string) => req<GrillSession>("GET", `/agents/${id}/grill`).catch(() => null),
  grillCommand: (sessionId: string, command: "send" | "decide" | "pause" | "resume" | "end", questionId?: string) =>
    req<{ ok: boolean; session: GrillSession }>("POST", `/grill/${sessionId}/command`, { command, question_id: questionId, client_id: crypto.randomUUID() }),
  /** Holds an answer to a round question (Loft doesn't run the agent until the round is sent). "Yes" accepts the recommendation. */
  /** Holds an answer to one question of the open round; with `discuss`, it is a comment for the agent instead of a decision. */
  grillAnswer: (agentId: string, sessionId: string, questionId: string, text: string, discuss = false) =>
    req<{ ok: boolean }>("POST", `/agents/${agentId}/messages`, { text, meta: { grill: { role: "answer", session_id: sessionId, question_id: questionId, discuss } } }),
  /** Asks a Planning agent for a grill; the backend opens the session. */
  grillStart: (agentId: string, text: string) =>
    req<{ ok: boolean }>("POST", `/agents/${agentId}/messages`, { text, client_id: crypto.randomUUID(), meta: { grill: { role: "start_request" } } }),
  restartRun: (id: string) => req("POST", `/agents/${id}/restart`),
  resetSession: (id: string) => req<Agent>("POST", `/agents/${id}/reset-session`),
  compact: (id: string, instructions: string) => req<{ ok: boolean }>("POST", `/agents/${id}/compact`, { instructions }),
  sessions: (id: string) => req<{ sessions: PastSession[]; current: string | null }>("GET", `/agents/${id}/sessions`),
  resumeSession: (id: string, sessionId: string) => req<Agent>("POST", `/agents/${id}/resume-session`, { session_id: sessionId }),
  randomizeAvatar: (id: string) => req<Agent>("POST", `/agents/${id}/avatar/randomize`),
  memories: (id: string) => req<Memory[]>("GET", `/agents/${id}/memories`),
  addMemory: (id: string, title: string, content = "", tags = "") => req<Memory>("POST", `/agents/${id}/memories`, { title, content, tags }),
  deleteMemory: (id: string, mid: number) => req("DELETE", `/agents/${id}/memories/${mid}`),
  /** Opens the workspace in VS Code; on a remote loft, via a Remote-SSH link handed to the local VS Code. */
  openEditor: async (id: string) => {
    const r = await req<{ ok: boolean; path: string; url?: string }>("POST", `/agents/${id}/open-editor`);
    if (r.url) window.location.href = r.url;
    return r;
  },
  files: (id: string) => req<FileEntry[]>("GET", `/agents/${id}/files`),
  /** One folder's entries; `path` starts at the agent's project folder (else its workspace), `../` allowed up to the
   *  projects folder. `dir` is where it landed, relative to the projects folder. */
  ls: (id: string, path: string) =>
    req<{ dir: string; entries: { name: string; dir: boolean }[] }>("GET", `/agents/${id}/ls?path=${encodeURIComponent(path)}`),
  permissions: (agentId?: string) => req<PermissionRequest[]>("GET", `/permissions${agentId ? `?agent_id=${agentId}` : ""}`),
  pullRequests: (agentId?: string) => req<PullRequest[]>("GET", `/pull-requests${agentId ? `?agent_id=${agentId}` : ""}`),
  refreshPullRequest: (id: string) => req<PullRequest>("POST", `/pull-requests/${id}/refresh`),
  mergePullRequest: (id: string, method: string) => req<PullRequest>("POST", `/pull-requests/${id}/merge`, { method }),
  decide: (id: string, decision: "allow_once" | "allow_always" | "deny", rule?: string) =>
    req<PermissionRequest>("POST", `/permissions/${id}`, { decision, rule }),
};
