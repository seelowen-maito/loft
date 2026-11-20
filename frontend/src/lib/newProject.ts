// The New project dialog on Repos: its name rules (the backend's `new_project::fix_name` and `name_problem`),
// which Lead plans it, what Create may do, and its Draft.

/** The name as typed, fixed as you type: lower case, and anything GitHub's repo names don't allow (spaces
 * included) becomes one `-`. While typing, a trailing `-` stays so `on ` can become `on-call`. */
export function fixName(raw: string, typing = true): string {
  let out = "";
  for (const ch of raw.toLowerCase()) {
    const c = /[a-z0-9._-]/.test(ch) ? ch : "-";
    if (!(c === "-" && out.endsWith("-"))) out += c;
  }
  out = out.replace(/^-+/, "");
  return typing ? out : out.replace(/-+$/, "");
}

/** Why a fixed name can't be a project folder: only what the fix can't repair. Same text as the backend. */
export function nameProblem(name: string): string | null {
  if (!name) return "Give it a name: letters, numbers, dots, dashes or underscores.";
  if (name.startsWith(".")) return "A name can't start with a dot.";
  if (name.includes("..")) return "A name can't contain \"..\".";
  if ([...name].length > 100) return "Keep the name under 100 characters.";
  return null;
}

/** Where it goes, as the user knows the projects folder (`~/projects`). */
export function projectPath(projectsDir: string | undefined, name: string): string {
  const root = (projectsDir || "~/projects").replace(/\/+$/, "").replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~");
  return `${root}/${name}`;
}

/** The "Try" chips: an example sentence for the idea, nothing else. Not stack templates. */
export const TRY_IDEAS = [
  { label: "Java + Vite", idea: "A Java service with a Vite frontend that tracks who is on call this week." },
  { label: "CLI", idea: "A small command-line tool that renames photos by the date they were taken." },
  { label: "iPhone app", idea: "An iPhone app that logs coffee I drink and shows a weekly chart." },
];

type Lead = { id: string; name: string; group?: string; role?: string };

/** The Leads, in team order (as the agent list comes). */
export const leadsOf = <T extends Lead>(agents: T[]): T[] => agents.filter((a) => a.role === "lead");

/** Who plans it by default: the Lead the user talked to last, otherwise the first Lead in team order. */
export function defaultLead(leads: Lead[], lastAgent: string | null): string | null {
  return leads.find((l) => l.id === lastAgent)?.id ?? leads[0]?.id ?? null;
}

/** The server's answer for a name as you type (`GET /api/projects/check`). */
export type NameCheck = { name: string; path: string; problem: { kind: "invalid" | "taken"; message: string } | null };

export type FormInput = {
  name: string;
  idea: string;
  lead: string | null;
  /** The check for this name, once it came back; null while it's on its way. */
  check: NameCheck | null;
  /** The name field was left, or Create was pressed: an invalid name may show red. */
  touched: boolean;
  creating: boolean;
};

export type FormView = {
  /** The red text under Name, with its invalid border; null shows none. */
  nameError: string | null;
  /** The name is someone else's folder already. */
  taken: string | null;
  canCreate: boolean;
};

/** What the form shows and whether Create works. Untouched fields never show red; a taken name shows at once. */
export function formView(f: FormInput): FormView {
  const name = fixName(f.name, false);
  const problem = name ? nameProblem(name) : null;
  const fresh = f.check && f.check.name === name ? f.check : null;
  const taken = fresh?.problem?.kind === "taken" ? fresh.problem.message : null;
  const serverInvalid = fresh?.problem?.kind === "invalid" ? fresh.problem.message : null;
  const invalid = problem ?? serverInvalid;
  return {
    nameError: f.touched && name ? invalid : null,
    taken,
    canCreate: !f.creating && !!name && !!f.idea.trim() && !!f.lead && !invalid && !taken && !!fresh,
  };
}

/** The error Callout: "Couldn't create <path>: <reason>. Nothing was created. <what to do>." */
export function createError(path: string, reason: string): string {
  const r = reason.trim().replace(/[.\s]+$/, "") || "something went wrong";
  const lower = r.toLowerCase();
  const todo = lower.includes("already exists")
    ? "Pick another name."
    : lower.includes("no lead")
      ? "Create a Lead, then come back."
      : lower.includes("failed to fetch") || lower.includes("network")
        ? "Check that Loft is running, then try again."
        : "Fix that, then try again.";
  // The server's own message may already say what to do: keep only its first sentence as the reason.
  const first = r.split(/\.\s/)[0];
  return `Couldn't create ${path}: ${first.charAt(0).toLowerCase() + first.slice(1)}. Nothing was created. ${todo}`;
}

/** The dialog's unsent text, kept per browser when it closes. Storage can throw when site data is blocked. */
export type ProjectDraft = { name: string; idea: string };
type Store = Pick<Storage, "getItem" | "setItem" | "removeItem">;
const KEY = "loft:draft:new-project";

export function loadProjectDraft(storage?: Store): ProjectDraft {
  try {
    const v = JSON.parse((storage ?? localStorage).getItem(KEY) ?? "null");
    if (v && typeof v.name === "string" && typeof v.idea === "string") return { name: v.name, idea: v.idea };
  } catch {
    /* unreadable */
  }
  return { name: "", idea: "" };
}

/** Saves the draft, or removes it when both fields are empty. */
export function saveProjectDraft(d: ProjectDraft, storage?: Store) {
  try {
    const s = storage ?? localStorage;
    if (!d.name.trim() && !d.idea.trim()) s.removeItem(KEY);
    else s.setItem(KEY, JSON.stringify(d));
  } catch {
    /* storage unavailable */
  }
}
