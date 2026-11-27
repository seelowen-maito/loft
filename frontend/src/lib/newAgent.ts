// The New agent dialog's form and its role rules, the same as in the agent's settings (#250).

export type NewAgentForm = {
  name: string;
  /** Its group, and its projects with the main one first. */
  group: string;
  projects: string[];
  role: string;
  personality: string;
  prompt: string;
  model: string;
  interval_minutes: number;
};

export const EMPTY_FORM: NewAgentForm = {
  name: "", group: "", projects: [], role: "custom", personality: "", prompt: "", model: "sonnet", interval_minutes: 0,
};

/** Fresh context for each Delivery ticket, as the backend's `agents::FRESH_CONTEXT`: set in the agent's settings, as a new agent starts Always (#495). */
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

/** An Agent template, as `GET /api/agent-templates` serves it: the same list `loft` creates agents from, its instructions filled in for the project asked for. */
export type AgentTemplate = {
  key: string;
  name: string;
  role: string;
  /** What it does, in a few words: its line in the role picker. */
  summary: string;
  /** What it does, in a sentence: the picker's footer. */
  about: string;
  personality: string;
  prompt: string;
  model: string;
  interval_minutes: number;
};

/** A line of the role picker: a team role (its template), a Custom template, or `custom`, the blank one. */
export type RolePick = { key: string; role: string; label: string; summary: string; about: string; template?: AgentTemplate };

/** The blank Custom line: no template, only the user's own instructions. */
export const BLANK_CUSTOM: RolePick = {
  key: "custom",
  role: "custom",
  label: "Custom",
  summary: "Follows only its own instructions",
  about: "A blank agent with no part in the team's work: you write what it does and when it wakes up.",
};

/** The team's roles, in the picker's order. */
const TEAM_ROLES = ["lead", "dev", "code_reviewer", "qa", "security"];

/**
 * The role picker's two sections: the team (Lead, Dev, Code reviewer, QA, Security), named by `labelOf(role)`, and the agents
 * that work on their own (blank Custom, then the Custom templates in the server's order), named after their template.
 */
export function rolePicks(templates: AgentTemplate[], labelOf: (role: string) => string): { team: RolePick[]; solo: RolePick[] } {
  const pick = (t: AgentTemplate, label: string): RolePick => ({ key: t.key, role: t.role, label, summary: t.summary, about: t.about, template: t });
  const team = TEAM_ROLES.flatMap((r) => templates.filter((t) => t.role === r).slice(0, 1).map((t) => pick(t, labelOf(r))));
  const solo = [BLANK_CUSTOM, ...templates.filter((t) => t.role === "custom").map((t) => pick(t, t.name))];
  return { team, solo };
}

/** The line the form stands on: its role's, or for Custom the template its instructions still are, else blank Custom. */
export function currentPick(f: NewAgentForm, picks: RolePick[]): RolePick {
  if (f.role !== "custom") return picks.find((p) => p.role === f.role) ?? BLANK_CUSTOM;
  return picks.find((p) => p.role === "custom" && p.template?.prompt === f.prompt) ?? BLANK_CUSTOM;
}

/** Whether a line can't be picked yet: Lead, until the agent has a group. */
export const pickLocked = (p: RolePick, f: NewAgentForm) => p.role === "lead" && !formTeam(f);

/** A line's number key in the role picker: 1 to 9, then 0 for the tenth line, none after. */
export const pickNumber = (i: number) => (i < 9 ? String(i + 1) : i === 9 ? "0" : "");

/**
 * What a key does in the role picker with line `at` highlighted: ↑↓ move the highlight and wrap around, a number jumps to
 * its line (one past the last does nothing), Enter picks the highlighted line unless it's locked. Anything else: nothing.
 */
export function pickerKey(at: number, rows: RolePick[], key: string, f: NewAgentForm): { at: number } | { pick: RolePick } | null {
  if (rows.length === 0) return null;
  if (key === "ArrowDown" || key === "ArrowUp") return { at: (at + (key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length };
  if (key === "Enter") {
    const p = rows[Math.min(at, rows.length - 1)];
    return pickLocked(p, f) ? null : { pick: p };
  }
  if (/^[0-9]$/.test(key)) {
    const i = key === "0" ? 9 : Number(key) - 1;
    return i < rows.length ? { at: i } : null;
  }
  return null;
}

/** The role picker's footer, about the highlighted line `at` (not the current one): why it's locked first, then what it does. */
export function pickerFooter(at: number, rows: RolePick[], f: NewAgentForm): string {
  const p = rows[Math.min(at, rows.length - 1)];
  return !p ? "" : `${pickLocked(p, f) ? "Only an agent with a group can be Lead: set its group first. " : ""}${p.about}`;
}

/**
 * The form once a line is picked: its role, and the template's name, instructions, personality, schedule and model, all
 * still editable. Blank Custom clears what a template filled in, and keeps a name the user typed.
 */
export function applyPick(f: NewAgentForm, p: RolePick, templates: AgentTemplate[]): NewAgentForm {
  const t = p.template;
  if (t) return { ...f, role: t.role, name: t.name, personality: t.personality, prompt: t.prompt, model: t.model, interval_minutes: t.interval_minutes };
  const fromTemplate = templates.some((x) => x.name === f.name);
  return { ...f, role: p.role, name: fromTemplate ? "" : f.name, personality: "", prompt: "", model: EMPTY_FORM.model, interval_minutes: 0 };
}

/** The form after the templates were fetched again for another project: instructions still a template's take the new project's. */
export function refillTemplate(f: NewAgentForm, before: AgentTemplate[], after: AgentTemplate[]): NewAgentForm {
  const key = before.find((t) => t.prompt === f.prompt)?.key;
  const t = key && after.find((x) => x.key === key);
  return t ? { ...f, prompt: t.prompt } : f;
}

/** Its team (`teamOf`): the group. */
const formTeam = (f: NewAgentForm) => f.group.trim();

/** The form with a new group or projects. Only an agent with a group can be Lead: without one it drops back to Custom. */
export const withTeam = (f: NewAgentForm, change: { group?: string; projects?: string[] }): NewAgentForm => {
  const next = { ...f, ...change };
  return { ...next, role: !formTeam(next) && next.role === "lead" ? "custom" : next.role };
};

/** What Create agent sends: the whole form, with a name even when left blank. No Fresh context: a new agent starts Always. */
export const createPayload = (f: NewAgentForm) => ({ ...f, name: f.name.trim() || "Unnamed" });
