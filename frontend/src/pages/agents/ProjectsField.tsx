import { useEffect, useState } from "react";
import { Star, X } from "lucide-react";
import { api } from "../../lib/api";
import { cx } from "../../lib/format";
import { withProject } from "../../lib/team";

// Repo names are fetched once per page load; the projects folder rarely changes.
let reposOnce: Promise<string[]> | null = null;
function repoNames(): Promise<string[]> {
  reposOnce ??= api.repos().then((rs) => rs.map((r) => r.name)).catch(() => []);
  return reposOnce;
}

/** An agent's group: free text, with the groups in use as suggestions. */
export function GroupInput({ value, groups, onChange }: { value: string; groups: string[]; onChange: (v: string) => void }) {
  return (
    <>
      <input className="input" list="agent-groups" value={value} placeholder="e.g. Acme" maxLength={100} onChange={(e) => onChange(e.target.value)} aria-label="Group" />
      <datalist id="agent-groups">{groups.map((g) => <option key={g} value={g} />)}</datalist>
    </>
  );
}

/**
 * An agent's projects as chips, main first with a filled star; another star makes that one the main project.
 * Add one from the projects in use and the git repos in the projects folder, or type a new name.
 */
export function ProjectsField({ value, projects, adding, onAdding, onChange }: { value: string[]; projects: string[]; adding: string; onAdding: (v: string) => void; onChange: (v: string[]) => void }) {
  const [repos, setRepos] = useState<string[]>([]);
  useEffect(() => {
    repoNames().then(setRepos);
  }, []);
  const choices = [...new Set([...projects, ...repos])].filter((p) => !value.includes(p)).sort((a, b) => a.localeCompare(b));
  const add = () => {
    onChange(withProject(value, adding));
    onAdding("");
  };

  return (
    <div className="space-y-2">
      {value.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {value.map((p, i) => (
            <span key={p} className={cx("inline-flex items-center gap-1 rounded-full border py-0.5 pl-1.5 pr-1 font-mono text-[11.5px]", i === 0 ? "border-accent/50 text-fg" : "border-line text-fg-2")}>
              <button
                type="button"
                onClick={() => onChange(withProject(value, p, true))}
                title={i === 0 ? "Main project: its issues go here by default" : "Make it the main project"}
                aria-label={i === 0 ? `${p} is the main project` : `Make ${p} the main project`}
                className="grid place-items-center"
              >
                <Star className={cx("size-3", i === 0 ? "fill-accent text-accent" : "text-fg-3 hover:text-fg")} />
              </button>
              {p}
              <button type="button" onClick={() => onChange(value.filter((x) => x !== p))} title={`Remove ${p}`} aria-label={`Remove ${p}`} className="grid place-items-center text-fg-3 hover:text-fg">
                <X className="size-3" />
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="flex gap-2">
        <input
          className="input"
          list="agent-project-choices"
          value={adding}
          placeholder={value.length ? "Add another project" : "Add a project (optional)"}
          onChange={(e) => onAdding(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), add())}
          aria-label="Add a project"
        />
        <datalist id="agent-project-choices">{choices.map((p) => <option key={p} value={p} />)}</datalist>
        <button type="button" className="btn shrink-0" onClick={add} disabled={!adding.trim()} title="Add the project">
          Add
        </button>
      </div>
      {adding.trim() && <p className="text-[12px] text-fg-3">Press Add to add it</p>}
    </div>
  );
}
