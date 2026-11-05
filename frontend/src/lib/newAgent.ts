// The New agent dialog's form and its role rules, the same as in the agent's settings (#250).

export type NewAgentForm = {
  name: string;
  title: string;
  project: string;
  role: string;
  fresh_context: string;
  personality: string;
  prompt: string;
  model: string;
  interval_minutes: number;
};

export const EMPTY_FORM: NewAgentForm = {
  name: "", title: "", project: "", role: "custom", fresh_context: "above_80k", personality: "", prompt: "", model: "sonnet", interval_minutes: 0,
};

export const TEMPLATES = [
  {
    label: "Repo janitor",
    name: "Moss",
    title: "Repo janitor",
    role: "custom",
    personality: "Calm, tidy, slightly dry. Speaks in short, precise sentences. Loves a clean working tree.",
    prompt:
      "Keep an eye on my repositories. On each wake-up, check for uncommitted work, stale branches, and repos that are behind their remote. Report anything worth my attention in a short bulleted list. Never push or modify my repos directly.",
    interval_minutes: 360,
  },
  {
    label: "Research scout",
    name: "Wren",
    title: "Research scout",
    role: "custom",
    personality: "Curious and enthusiastic, but ruthless about signal over noise. Always cites sources.",
    prompt:
      "Track new developments relevant to the languages and frameworks used in my projects. Keep running notes in notes/ in your workspace, and remember what you've already reported so you never repeat yourself.",
    interval_minutes: 1440,
  },
  {
    label: "Pair programmer",
    name: "Bolt",
    title: "Pair programmer",
    role: "dev",
    personality: "Direct, pragmatic senior engineer. Proposes a plan before big changes and explains trade-offs briefly.",
    prompt: "Help me with coding tasks on request. Clone repos into your workspace, make changes on a branch, run the tests, and summarize the diff.",
    interval_minutes: 0,
  },
];

/** The form with its project set to `project`. Only an agent with a project can be Lead: without one it drops back to Custom. */
export const withProject = (f: NewAgentForm, project: string): NewAgentForm => ({
  ...f,
  project,
  role: !project.trim() && f.role === "lead" ? "custom" : f.role,
});

/** Whether the dialog offers Fresh context: to every role but a Lead that only coordinates, i.e. whose project has a Dev. */
export const offersFreshContext = (f: NewAgentForm, agents: { role?: string; project: string }[]) =>
  f.role !== "lead" || !agents.some((a) => a.role === "dev" && a.project.trim() === f.project.trim());

/** What Create agent sends: the whole form, role and Fresh context included, with a name even when left blank. */
export const createPayload = (f: NewAgentForm) => ({ ...f, name: f.name.trim() || "Unnamed" });
