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
  kind: "text" | "thinking" | "tool_use" | "tool_result" | "permission" | "error" | "notice" | "run_start" | "run_end" | "clear" | "compact";
  content: string;
  meta: Record<string, any>;
  created_at: string;
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

/** An interview (grill) the agent is running; `GET /agents/{id}/grill`. */
export type GrillSession = {
  session_id: string;
  agent_id: string;
  state: "preparing" | "active" | "paused" | "ended";
  end_reason: string | null;
  topic: string;
  index: number | null;
  total: number | null;
  answered: number;
  current: { question_id: string; index: number | null } | null;
  started_at: string;
  updated_at: string;
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

export type Memory = { id: number; agent_id: string; content: string; tags: string; created_at: string };

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
  imessage: {
    enabled: boolean;
    to: string;
    /** Agent that receives iMessage replies; "" ignores them. */
    agent_id: string;
    permission: boolean;
    scheduled_run: boolean;
    manual_run: boolean;
    pr_opened: boolean;
    pr_merged: boolean;
    update: boolean;
  };
};

export type SettingsView = {
  settings: Settings;
  github_token_set: boolean;
  /** Fields that only apply after a restart. */
  restart_fields: string[];
  /** Restart-only fields saved but not yet in effect. */
  pending_restart: string[];
  running_agents: number;
  /** Why iMessage replies can't be read, e.g. missing Full Disk Access. */
  imessage_inbox_error: string | null;
};

/** Claude plan limits, as in Claude Code's /usage. percent is 0–100. */
export type UsageWindow = { key: string; label: string; percent: number; resets_at: string | null };
export type Usage = { windows: UsageWindow[]; error: string | null };

/** The Mac's hottest CPU die sensor, sampled every 30 s. Null where it can't be read. */
export type SystemReading = { cpu_temp_c: number | null; level: "nominal" | "warm" | "hot" | null; sampled_at: string | null };

export type Info = { projects_dir: string; storage_dir: string; github_token: boolean; version: string };

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
  testImessage: (to: string) => req<{ ok: boolean }>("POST", "/settings/imessage-test", { to }),
  restart: () => req<{ ok: boolean }>("POST", "/restart"),
  repos: (refresh = false) => req<Repo[]>("GET", `/repos${refresh ? "?refresh=true" : ""}`),
  github: (refresh = false) => req<Record<string, GithubInfo>>("GET", `/repos/github${refresh ? "?refresh=true" : ""}`),
  commits: (name: string) => req<Commit[]>("GET", `/repos/${encodeURIComponent(name)}/commits`),

  agents: () => req<Agent[]>("GET", "/agents"),
  agentsSummary: () => req<AgentSummary[]>("GET", "/agents/summary"),
  createAgent: (a: Partial<Agent>) => req<Agent>("POST", "/agents", a),
  reorderAgents: (ids: string[]) => req<{ ok: boolean }>("PUT", "/agents/order", { ids }),
  updateAgent: (id: string, a: Partial<Agent>) => req<Agent>("PATCH", `/agents/${id}`, a),
  deleteAgent: (id: string, purge: boolean) => req<{ ok: boolean }>("DELETE", `/agents/${id}?purge_files=${purge}`),
  messages: (id: string) => req<Message[]>("GET", `/agents/${id}/messages?limit=400`),
  /** `taskId` attaches a task: the agent gets its title, status and latest note with the message. */
  send: (id: string, text: string, images: string[] = [], taskId?: string) =>
    req<{ ok: boolean; queued: boolean }>("POST", `/agents/${id}/messages`, { text, images, ...(taskId ? { meta: { task_id: taskId } } : {}) }),
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
  grillCommand: (sessionId: string, command: "pause" | "resume" | "end") =>
    req<{ ok: boolean; session: GrillSession }>("POST", `/grill/${sessionId}/command`, { command, client_id: crypto.randomUUID() }),
  restartRun: (id: string) => req("POST", `/agents/${id}/restart`),
  resetSession: (id: string) => req<Agent>("POST", `/agents/${id}/reset-session`),
  compact: (id: string, instructions: string) => req<{ ok: boolean }>("POST", `/agents/${id}/compact`, { instructions }),
  randomizeAvatar: (id: string) => req<Agent>("POST", `/agents/${id}/avatar/randomize`),
  memories: (id: string) => req<Memory[]>("GET", `/agents/${id}/memories`),
  addMemory: (id: string, content: string, tags = "") => req<Memory>("POST", `/agents/${id}/memories`, { content, tags }),
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
