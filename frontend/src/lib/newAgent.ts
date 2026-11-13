// The New agent dialog's form and its role rules, the same as in the agent's settings (#250).

export type NewAgentForm = {
  name: string;
  /** Its group, and its projects with the main one first. */
  group: string;
  projects: string[];
  role: string;
  fresh_context: string;
  personality: string;
  prompt: string;
  model: string;
  interval_minutes: number;
};

export const EMPTY_FORM: NewAgentForm = {
  name: "", group: "", projects: [], role: "custom", fresh_context: "always", personality: "", prompt: "", model: "sonnet", interval_minutes: 0,
};

/** Fresh context for each Delivery ticket, as the backend's `agents::FRESH_CONTEXT`. A new agent starts Always. */
export const FRESH_CONTEXT = [
  { value: "always", label: "Always (default)" },
  { value: "above_80k", label: "Above 80k" },
  { value: "above_100k", label: "Above 100k" },
  { value: "above_120k", label: "Above 120k" },
  { value: "above_150k", label: "Above 150k" },
  { value: "above_180k", label: "Above 180k" },
  { value: "above_200k", label: "Above 200k" },
  { value: "off", label: "Off" },
];

export const TEMPLATES = [
  {
    label: "Repo janitor",
    name: "Moss",
    role: "custom",
    personality: "Calm, tidy, slightly dry. Speaks in short, precise sentences. Loves a clean working tree.",
    prompt:
      "Keep an eye on my repositories. On each wake-up, check for uncommitted work, stale branches, and repos that are behind their remote. Report anything worth my attention in a short bulleted list. Never push or modify my repos directly.",
    interval_minutes: 360,
  },
  {
    label: "Research scout",
    name: "Wren",
    role: "custom",
    personality: "Curious and enthusiastic, but ruthless about signal over noise. Always cites sources.",
    prompt:
      "Track new developments relevant to the languages and frameworks used in my projects. Keep running notes in notes/ in your workspace, and remember what you've already reported so you never repeat yourself.",
    interval_minutes: 1440,
  },
  {
    label: "Pair programmer",
    name: "Bolt",
    role: "dev",
    personality: "Direct, pragmatic senior engineer. Proposes a plan before big changes and explains trade-offs briefly.",
    prompt: "Help me with coding tasks on request. Clone repos into your workspace, make changes on a branch, run the tests, and summarize the diff.",
    interval_minutes: 0,
  },
];

/** The template the form was started from, while its instructions are still the template's: the one the dialog highlights. */
export const templateOf = (f: NewAgentForm) => TEMPLATES.find((t) => t.prompt === f.prompt);

/** Its team (`teamOf`): the group. */
const formTeam = (f: NewAgentForm) => f.group.trim();

/** The form with a new group or projects. Only an agent with a group can be Lead: without one it drops back to Custom. */
export const withTeam = (f: NewAgentForm, change: { group?: string; projects?: string[] }): NewAgentForm => {
  const next = { ...f, ...change };
  return { ...next, role: !formTeam(next) && next.role === "lead" ? "custom" : next.role };
};

/** Whether the dialog offers Fresh context: to every role but a Lead that only coordinates, i.e. whose team has a Dev. */
export const offersFreshContext = (f: NewAgentForm, agents: { role?: string; project: string; group?: string }[]) =>
  f.role !== "lead" || !agents.some((a) => a.role === "dev" && a.group?.trim() === formTeam(f));

/** What Create agent sends: the whole form, role and Fresh context included, with a name even when left blank. */
export const createPayload = (f: NewAgentForm) => ({ ...f, name: f.name.trim() || "Unnamed" });
