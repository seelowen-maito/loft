export type Avatar = { svg: string };

/** A weekly window with no autonomous wake-ups, in the Mac's local time. Days: 0 = Monday … 6 = Sunday. */
export type QuietWindow = { days: number[]; start: string; end: string };

export type Agent = {
  id: string;
  name: string;
  title: string;
  /** Its main project, where its issues go by default: the first of `projects`. */
  project: string;
  /** Free-text group its team goes by (Acme); "" means no team. Missing from servers before groups. */
  group?: string;
  /** The projects it works in, main first. Missing from servers before groups. */
  projects?: string[];
  /** Projects that are a folder of repos or a missing folder: kept, but the user picks again. */
  project_problems?: { project: string; problem: string }[];
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
  /** What it does in its team: one of `ROLES`. A Lead (one per group, `teamOf`) wears a captain hat. */
  role?: string;
  /** Fresh context for each Delivery ticket: "off", "above_80k" (the default), "above_100k" … "above_200k" or "always". */
  fresh_context?: string;
  /** Whether that setting applies: to every agent but a Lead that only coordinates (its team has a Dev). */
  fresh_context_applies?: boolean;
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
  /** Subagents the agent itself started in the current run that haven't returned yet. */
  subagents?: RunningSubagent[];
  /** A run the user started (by a message or a schedule) ended with a reply or an error they haven't seen. */
  unread_reply?: boolean;
  /** Agent messages in its chat (their ids) waiting for its current run to end. */
  queued_messages?: number[];
};

/** The roles an agent can have, as stored in `Agent.role`, with what each does. */
export const ROLES: { value: string; label: string; hint: string }[] = [
  { value: "lead", label: "Lead", hint: "plans with you" },
  { value: "dev", label: "Dev", hint: "builds the tickets" },
  { value: "code_reviewer", label: "Code reviewer", hint: "reviews the code" },
  { value: "qa", label: "QA", hint: "tests the work" },
  { value: "security", label: "Security", hint: "checks for security problems" },
  { value: "custom", label: "Custom", hint: "follows only its own instructions" },
];

export type RunningSubagent = { id: string; description: string; type: string; started_at: string };

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
  /** The agent that sent `first_message`, when another agent started the Session. */
  from_agent: string | null;
  context_tokens: number | null;
};

export type TaskStatus = "inbox" | "ready" | "in_progress" | "blocked" | "waiting_approval" | "done" | "failed" | "cancelled";

/** A task on the shared list (`/api/tasks`). */
/** A Loft issue (stored as a task): shaped like a GitHub issue, plus an agent, an execution status and a thread. */
/** A Delivery as an issue page shows it. A spec's also lists its tickets: "elsewhere" ones are being built
 *  outside it (`skipped` says why), so it never hands them out. */
export type IssueDelivery = {
  /** "planned", "running", "final_review", "final_fixes", "needs_human", "stalled", "paused", "ready", "settled" or "merged". */
  state: string;
  /** Why it stalled: "run_failed", "no_call" or "restart"; null unless stalled. */
  stall_reason?: string | null;
  /** The card's state line as the server words it; null before it started. An older server doesn't send it. */
  state_line?: { tone: string; text: string } | null;
  /** Pause applies: the Delivery is running (its final review and fixes included), and no ticket waits for the user. */
  can_pause?: boolean;
  /** Resume applies: the Delivery is stalled or paused, or a ticket or its final review waits for the user. */
  can_resume?: boolean;
  /** A ticket, or the final review, waits for the user after too many changes rounds. */
  needs_human?: boolean;
  started: boolean;
  agent_id: string | null;
  agent_name: string | null;
  pr: number | null;
  spec?: boolean;
  current?: { number: number; title: string } | null;
  tickets?: { number: number; title: string; state: string; skipped: string; agent_id?: string | null; agent_name?: string | null }[];
  /** With a Lead on the team: the ready tickets nobody has yet, while the Lead hands them out. */
  waiting_for_lead?: number[];
  /** Tickets that had too many rounds of changes: no agent runs them until the user resumes. */
  waiting_on_you?: number[];
  /** Its open PR's branch is behind its base by `count` commits; null when it isn't. An older server doesn't send it. */
  behind?: { count: number; base: string } | null;
};

export type Task = {
  id: string;
  title: string;
  /** Markdown. */
  description: string;
  /** Label names, as on GitHub. */
  labels: string[];
  /** GitHub login of the person who owns it, or "". */
  owner: string;
  /** Derived from status: done and cancelled are closed. Read-only. */
  state: "open" | "closed";
  /** The GitHub issue it mirrors, or null for a Loft-only issue. `state` is GitHub's. */
  github: { repo: string; number: number; url: string; state: "open" | "closed"; updated_at: string; sub_total?: number; sub_done?: number; /** who opened it on GitHub */ author?: string } | null;
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
  meta: {
    author_name?: string; status?: string; summary?: string; error?: string; private?: boolean; to_agent?: string; client_id?: string;
    /** On a mirrored issue: pending (waits for your approval), posted, kept (in Loft only), or from_github. */
    github?: { state: "pending" | "posted" | "kept" | "from_github"; comment_id?: number; auto?: boolean };
  };
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
  /** Uploads sent with a typed answer or comment. Missing from older Loft versions. */
  images?: string[];
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
  /** Left in the composer after Clear context, never sent by Loft. Empty for a spec with a Delivery. */
  next_message: string;
  /** The spec's Delivery, for Start delivery; `started` once it started: who got which ticket. */
  delivery?: { repo: string; spec: number; standalone?: boolean; local?: boolean; started?: DeliveryStarted } | null;
  /** Roles no agent in the project fills: "dev", "code_reviewer", "qa", "security". */
  missing_roles: string[];
  /** The message "Set up the team with lofty" sends, when roles are missing; `sent_at` once it was sent. */
  setup: { agent_id: string; message: string; sent_at?: string } | null;
};

export type DeliveryStarted = { ticket: number; title?: string; agent_id: string; agent_name: string; at: string };

/** A browser or iPhone paired with Loft (`/api/paired-clients`); `this` is the one asking. */
export type PairedClient = { id: string; name: string; paired_at: string; last_used_at: string; this: boolean };

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

/** The relay's portal page where the user manages their keys, when Sign in is offered; else null (only pasting a key). */
export const portalHome = (s: Pick<PushStatus, "relay_url" | "signin">): string | null => (s.signin && s.relay_url ? `${s.relay_url}/` : null);

/** `GET /api/push`: which path pushes take: `own_key` (straight to Apple), `relay` (the Loft push relay) or `none`. Never the key or the relay key. */
export type PushStatus = {
  path: "own_key" | "relay" | "none";
  /** Whether a relay key is stored and the relay hasn't refused it. */
  relay_key: boolean;
  /** The relay answered 401 to the stored key: pushes through it stopped until the user signs in again or pastes a new key. */
  relay_refused: boolean;
  /** Sign in was pressed and the code isn't allowed on the portal yet. */
  signing_in: boolean;
  /** While Sign in waits: the confirmation code the user types on the portal's Allow page (null from a relay that gives none). */
  signin_code: string | null;
  /** The relay address in use (saved, else LOFT_RELAY_URL, else the official relay); null when there is none. Its push portal is at the same address. */
  relay_url: string | null;
  /** The address saved in Settings ("" when none: the environment's, if any, is used). */
  relay_address: string;
  /** Sign in is offered: there is a relay, and it hasn't said its portal isn't configured. */
  signin: boolean;
  configured: boolean;
  key_present: boolean;
  key_id: string;
  team_id: string;
  bundle_id: string;
  devices: number;
};

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
  /** Label names, as GitHub has them (the verdicts are tests:green, qa:passed, security:passed and their failures). */
  labels?: string[];
  /** Commits on the base the branch doesn't have; 0 when up to date. An older server doesn't send it. */
  behind_by?: number;
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

/** The home line's figures next to the per-agent summaries (`GET /agents/summary?totals=true`). */
export type AgentsSummary = {
  agents: AgentSummary[];
  /** Runs and estimated cost of the current Usage week (from the weekly reset minus 7 days), or of the last 7 days when the plan usage can't be read. */
  period: {
    kind: "usage_week" | "last_7_days";
    since: string;
    resets_at: string | null;
    runs: number;
    cost_usd: number;
    /** The span before, like with like: the previous usage week up to the same point in it, or the 7 days before the last 7. */
    previous: { since: string; until: string; runs: number; cost_usd: number };
  };
  /** The All-time spend: every run since Loft started counting, deleted agents and cleared chats included. */
  all_time: { runs: number; cost_usd: number };
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
  messaging_rules: boolean;
  /** Publish new Loft issues to GitHub as they're made (projects with a GitHub repo). */
  auto_publish_all: boolean;
  /** How comments on GitHub issues are approved, for every project: ask (every one waits), own (yours go straight) or all. */
  comment_approval_default: ApprovalMode;
  /** Where the daily backups go; "" is ~/Library/Application Support/Loft/backups. */
  backup_dir: string;
};

/** The backups folder, its newest backup and the last attempt's error (a good backup clears it). */
export type BackupStatus = {
  dir: string;
  last: { at: string; bytes: number; path: string } | null;
  error: { message: string; at: string } | null;
  /** iCloud Drive → Loft backups, the one-click choice; null when iCloud Drive is off on the Mac. */
  icloud: string | null;
};

/** Every agent's workspace size and whether it is busy (running or a live dev server: Clean skips it), and the disk's
 * free space. */
export type WorkspaceSizes = {
  agents: { id: string; name: string; bytes: number; busy: boolean }[];
  free_bytes: number | null;
  low_disk_bytes: number;
};
/** What a clean leaves, and why; `rel` is inside the agent's workspace, empty for all of it. */
export type CleanSkip = { agent: string; rel: string; reason: string };
/** One thing Clean lists: a workspace's top-level item, or a folder inside a main clone. `path` is absolute; `command`
 * is the equivalent `rm -rf`, to read or copy (Loft removes by path); `changed` is its newest file's change. `label`
 * says why it starts unticked, `blocked` why it can't be ticked, and `opens` that it lists its own folders. */
export type CleanItem = {
  agent: string;
  path: string;
  rel: string;
  name: string;
  kind: "clone" | "worktree" | "folder" | "file" | "symlink";
  bytes: number;
  changed: string;
  command: string;
  label: string;
  note: string;
  warning: string;
  ticked: boolean;
  blocked: string;
  opens: boolean;
};
/** A clean's log, live while `running`: each removal (or its error), what was skipped, and the totals. */
export type CleanLog = {
  at: string | null;
  what: string;
  /** "Cleaning Loft Tester…" while running, then "Cleaned Loft Tester"; empty in an older log. */
  title?: string;
  running: boolean;
  lines: { agent: string; path: string; rel: string; bytes: number; error: string | null; command: string }[];
  skipped: CleanSkip[];
  freed_bytes: number;
  removed: number;
  failed: number;
};

export type ApprovalMode = "ask" | "own" | "all";

/** GitHub's rate limit as its last answers said (`reset` in Unix seconds), and Loft's requests this hour by source. */
export type GithubBudget = {
  resources: Record<string, { limit: number; remaining: number; used: number; reset: number }>;
  by_source: Record<string, { requests: number; not_modified: number }>;
  window_reset: number | null;
};

/** What reading GitHub just did: per mirrored project, and why the others aren't mirrored. */
export type MirrorSync = {
  connected: boolean;
  /** The GitHub user of Loft's token; null when nothing is mirrored or it can't be read. */
  login?: string | null;
  projects: { project: string; repo: string; created: number; updated: number; error?: string }[];
  skipped: { project: string; reason: string }[];
};

/** Whether macOS lets Loft into each guarded folder; `waiting`: a prompt is up on the Mac now. */
export type MacAccess = {
  items: { id: string; label: string; folder: string; state: "allowed" | "blocked" | "waiting" | "missing" }[];
  binary: string | null;
  adhoc_signed: boolean | null;
  host: string | null;
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

/** Fired when Loft refuses a request because this browser is not paired (or was unpaired). */
export const UNPAIRED = "loft:unpaired";

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body !== undefined ? { "content-type": "application/json" } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (res.status === 401) window.dispatchEvent(new Event(UNPAIRED));
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

/** A booted simulator; `agent` is the agent whose device it is, null for one the user booted. */
export type Simulator = { udid: string; name: string; runtime: string; agent: { id: string; name: string } | null };
/** Points are fractions of the screen, 0–1. */
export type SimulatorInput =
  | { action: "tap"; x: number; y: number }
  | { action: "swipe"; x: number; y: number; x2: number; y2: number; duration: number }
  | { action: "type"; text: string }
  | { action: "key"; code: number }
  | { action: "button"; name: "home" | "lock" };

export const api = {
  simulators: () => req<{ interactive: boolean; devices: Simulator[]; error?: string }>("GET", "/simulator"),
  simulatorInput: (udid: string, input: SimulatorInput) => req<{ ok: boolean }>("POST", "/simulator/input", { udid, ...input }),
  info: () => req<Info>("GET", "/info"),
  update: () => req<UpdateStatus>("GET", "/update"),
  usage: () => req<Usage>("GET", "/usage"),
  system: () => req<SystemReading>("GET", "/system"),
  /** Lists a fixed set of macOS-guarded folders; can show macOS's prompts on the Mac. */
  macAccess: () => req<MacAccess>("POST", "/system/mac-access"),
  settings: () => req<SettingsView>("GET", "/settings"),
  /** github_token: undefined keeps the saved token, "" removes it. */
  saveSettings: (s: Omit<Settings, "github_token" | "backup_dir"> & { github_token?: string; backup_dir?: string }) => req<SettingsView>("PUT", "/settings", s),
  restart: () => req<{ ok: boolean }>("POST", "/restart"),
  backups: () => req<BackupStatus>("GET", "/backups"),
  workspaces: () => req<WorkspaceSizes>("GET", "/workspaces"),
  /** What Clean on one agent, or Clean all, lists: each workspace's top-level items. */
  cleanPreview: (agent?: string) => req<{ items: CleanItem[] }>("GET", agent ? `/workspaces/clean/preview?agent=${encodeURIComponent(agent)}` : "/workspaces/clean/preview"),
  /** The folders in `rel` inside one of the agent's main clones. */
  cleanFolders: (agent: string, rel: string) => req<{ items: CleanItem[] }>("GET", `/workspaces/clean/folders?agent=${encodeURIComponent(agent)}&rel=${encodeURIComponent(rel)}`),
  /** Removes the previewed folders the user kept ticked. */
  clean: (agent: string | undefined, paths: string[]) => req<CleanLog>("POST", "/workspaces/clean", { agent, paths }),
  /** The running or last clean's log, the daily sweep's included. */
  cleanLog: () => req<CleanLog>("GET", "/workspaces/log"),
  backUpNow: () => req<BackupStatus>("POST", "/backups"),
  githubBudget: () => req<GithubBudget>("GET", "/github/budget"),
  repos: (refresh = false) => req<Repo[]>("GET", `/repos${refresh ? "?refresh=true" : ""}`),
  github: (refresh = false) => req<Record<string, GithubInfo>>("GET", `/repos/github${refresh ? "?refresh=true" : ""}`),
  commits: (name: string) => req<Commit[]>("GET", `/repos/${encodeURIComponent(name)}/commits`),

  agents: () => req<Agent[]>("GET", "/agents"),
  /** Only what the ⌘J shortcut reads; see docs/needs-you-api.md for the whole item. */
  needsYou: () => req<{ items: { agent: { id: string; name: string } | null }[] }>("GET", "/needs-you"),
  /** New devices asking to pair, from Needs you (the `pairing` items). */
  pairRequests: () =>
    req<{ items: { type: string; text: string; actions: { kind: string; path: string; body: { code: string } }[] }[] }>("GET", "/needs-you").then((r) =>
      r.items.filter((i) => i.type === "pairing").map((i) => ({ code: i.actions[0].body.code, text: i.text })),
    ),
  answerPairing: (code: string, allow: boolean) => req<{ ok: boolean }>("POST", `/pair/${allow ? "approve" : "deny"}`, { code }),
  agentsSummary: () => req<AgentsSummary>("GET", "/agents/summary?totals=true"),
  createAgent: (a: Partial<Agent>) => req<Agent & { notice?: string }>("POST", "/agents", a),
  reorderAgents: (ids: string[]) => req<{ ok: boolean }>("PUT", "/agents/order", { ids }),
  updateAgent: (id: string, a: Partial<Agent>) => req<Agent & { notice?: string }>("PATCH", `/agents/${id}`, a),
  /** What deleting the agent with its workspace would free. */
  workspaceSize: (id: string) => req<{ bytes: number }>("GET", `/agents/${id}/workspace-size`),
  deleteAgent: (id: string, purge: boolean) => req<{ ok: boolean }>("DELETE", `/agents/${id}?purge_files=${purge}`),
  messages: (id: string) => req<Message[]>("GET", `/agents/${id}/messages?limit=400`),
  /** `taskId` attaches a task: the agent gets its title, status and latest note with the message. */
  send: (id: string, text: string, images: string[] = [], taskId?: string) =>
    req<{ ok: boolean; queued: boolean }>("POST", `/agents/${id}/messages`, { text, images, ...(taskId ? { meta: { task_id: taskId } } : {}) }),
  /** "Set up the team with lofty" on an end-of-planning card: sends its setup message once. */
  sendTeamSetup: (id: string, messageId: number) =>
    req<{ ok: boolean; duplicate?: boolean; message: Message }>("POST", `/agents/${id}/messages/${messageId}/team-setup`),
  /** "Start delivery" on an end-of-planning card: starts the spec's first ready ticket on the team's Dev, once. */
  startDelivery: (id: string, messageId: number, initGit = false) =>
    req<{ ok: boolean; duplicate?: boolean; needs_git?: boolean; dir?: string; message: Message }>("POST", `/agents/${id}/messages/${messageId}/start-delivery${initGit ? "?init_git=true" : ""}`),
  /** Where a Delivery stands: for a plan kept in Loft, "ready" means Merge can run. */
  deliveryState: (repo: string, spec: number) =>
    req<{ state: string; local: boolean; branch: string; base: string }>("GET", `/deliveries/state?repo=${encodeURIComponent(repo)}&spec=${spec}`),
  /** Merge for a plan kept in Loft: its spec branch into the project's checked-out base, when the tree is clean. */
  mergeDelivery: (repo: string, spec: number) => req<{ ok: boolean; message: string }>("POST", "/deliveries/merge", { repo, spec }),
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
  /** The user has seen this chat: clears its Unread reply in every browser. */
  markRead: (id: string) => req<Agent>("POST", `/agents/${id}/read`),
  /** Stops a run that waits or went quiet; queued messages are kept and run next. */
  tasks: () => req<Task[]>("GET", "/issues"),
  /** Reads the projects' GitHub issues now; new and changed ones arrive as task events. */
  syncIssues: () => req<MirrorSync>("POST", "/issues/sync", {}),
  task: (id: string) => req<Task>("GET", `/issues/${id}`),
  pushStatus: () => req<PushStatus>("GET", "/push"),
  configurePush: (b: { key_id?: string; team_id?: string; bundle_id?: string; key_p8?: string; remove_key?: boolean; relay_key?: string; remove_relay_key?: boolean; relay_url?: string }) =>
    req<PushStatus>("PUT", "/push", b),
  /** Asks the relay for a one-time code; `url` is the portal page that allows it. The key is collected by the backend. */
  pushSignIn: () => req<{ url: string; confirm: string | null }>("POST", "/push/signin"),
  devices: () => req<PushDevice[]>("GET", "/devices"),
  testPush: (token: string) => req<{ ok: boolean }>("POST", `/devices/${token}/test`),
  removeDevice: (token: string) => req<{ ok: boolean }>("DELETE", `/devices/${token}`),
  pairedClients: () => req<PairedClient[]>("GET", "/paired-clients"),
  /** Forget this device: that client is refused from its next request on. */
  forgetClient: (id: string) => req<{ ok: boolean }>("DELETE", `/paired-clients/${id}`),
  /** Unpairs every client but this one. */
  unpairAll: () => req<{ unpaired: number }>("POST", "/paired-clients/unpair-all", {}),
  taskPlan: (id: string) => req<TaskPlan>("GET", `/issues/${id}/plan`),
  taskComments: (id: string) => req<TaskComment[]>("GET", `/issues/${id}/comments?limit=200`),
  /** `send`: deliver to the task's agent (else a private note); `github`: post it on the mirrored issue too, after approval;
   * `reopen`: run a finished task again with it (a failed one always runs again); `agent_id`: on a task with nobody on it,
   * hand it to this agent. */
  commentTask: (id: string, text: string, opts: { send: boolean; github: boolean; reopen: boolean; agent_id?: string }) =>
    req<{ comment: TaskComment | null; queued?: boolean; delivered_to?: string | null; task?: Task }>("POST", `/issues/${id}/comments`, {
      text,
      client_id: crypto.randomUUID(),
      ...opts,
    }),
  createTask: (t: Partial<Task> & { title: string }) => req<Task>("POST", "/issues", { ...t, id: crypto.randomUUID() }),
  updateTask: (id: string, p: Partial<Omit<Task, "owner">> & { owner?: string | null }) => req<Task>("PATCH", `/issues/${id}`, p),
  runTask: (id: string, agentId?: string, note = "") => req<{ task: Task; queued: boolean }>("POST", `/issues/${id}/execute`, { agent_id: agentId, note }),
  cancelTask: (id: string, stopRun: boolean) => req<{ task: Task; stopped: boolean }>("POST", `/issues/${id}/cancel`, { stop_run: stopRun }),
  deleteTask: (id: string) => req<{ ok: boolean }>("DELETE", `/issues/${id}`),
  /** Publish to GitHub: creates the GitHub issue for a Loft-only issue and links them. */
  publishTask: (id: string) => req<Task>("POST", `/issues/${id}/publish`),
  /** Closed on GitHub while being worked on: go on, and reopen it there. */
  keepWorking: (id: string) => req<Task>("POST", `/issues/${id}/keep-working`, {}),
  /** A standalone ticket's or a spec's Delivery: whether it started, who has it, its PR and a spec's tickets; null when it has none. */
  /** Every spec's Delivery in one read, for the Issues page's spec views. */
  deliverySummaries: () => req<import("./specViews").DeliverySummary[]>("GET", "/deliveries/summary"),
  issueDelivery: (id: string) => req<IssueDelivery | null>("GET", `/issues/${id}/delivery`),
  /** "Start delivery" on a standalone ticket or a spec: its branch, its draft PR and the team's Dev, once. */
  startIssueDelivery: (id: string) => req<IssueDelivery & { ok: boolean; duplicate: boolean; ticket: number }>("POST", `/issues/${id}/start-delivery`),
  /** Resume on a stalled or paused Delivery: Loft checks what was done meanwhile, then wakes only the agent its step waits on. */
  resumeIssueDelivery: (id: string) => req<IssueDelivery & { ok: boolean; message: string }>("POST", `/issues/${id}/resume-delivery`),
  /** Pause on a running Delivery: Loft stops the run working on it and sends no more wakes until Resume. */
  pauseIssueDelivery: (id: string) => req<IssueDelivery & { ok: boolean; message: string }>("POST", `/issues/${id}/pause-delivery`),
  /** Posts a Loft comment on the GitHub issue, with `text` instead when you edited it. */
  approveComment: (id: string, cid: number, text?: string) => req<TaskComment>("POST", `/issues/${id}/comments/${cid}/approve`, text === undefined ? {} : { text }),
  keepComment: (id: string, cid: number) => req<TaskComment>("POST", `/issues/${id}/comments/${cid}/keep`, {}),
  /** The agent's open interview, or null. */
  grill: (id: string) => req<GrillSession>("GET", `/agents/${id}/grill`).catch(() => null),
  grillCommand: (sessionId: string, command: "send" | "decide" | "pause" | "resume" | "end", questionId?: string) =>
    req<{ ok: boolean; session: GrillSession }>("POST", `/grill/${sessionId}/command`, { command, question_id: questionId, client_id: crypto.randomUUID() }),
  /** Holds an answer to a round question (Loft doesn't run the agent until the round is sent). "Yes" accepts the recommendation. */
  /** Holds an answer to one question of the open round; with `discuss`, it is a comment for the agent instead of a decision. */
  grillAnswer: (agentId: string, sessionId: string, questionId: string, text: string, discuss = false, images: string[] = []) =>
    req<{ ok: boolean }>("POST", `/agents/${agentId}/messages`, { text, images, meta: { grill: { role: "answer", session_id: sessionId, question_id: questionId, discuss } } }),
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
  /** Up to 20 files and folders under the agent's `@` base whose path fuzzy-matches `q`, best first, with paths
   *  relative to the base. In a git repo, only what git tracks or doesn't ignore. */
  findFiles: (id: string, q: string) => req<{ path: string; dir: boolean }[]>("GET", `/agents/${id}/find?q=${encodeURIComponent(q)}`),
  permissions: (agentId?: string) => req<PermissionRequest[]>("GET", `/permissions${agentId ? `?agent_id=${agentId}` : ""}`),
  pullRequests: (agentId?: string) => req<PullRequest[]>("GET", `/pull-requests${agentId ? `?agent_id=${agentId}` : ""}`),
  refreshPullRequest: (id: string) => req<PullRequest>("POST", `/pull-requests/${id}/refresh`),
  mergePullRequest: (id: string, method: string) => req<PullRequest>("POST", `/pull-requests/${id}/merge`, { method }),
  decide: (id: string, decision: "allow_once" | "allow_always" | "deny", rule?: string) =>
    req<PermissionRequest>("POST", `/permissions/${id}`, { decision, rule }),
};
