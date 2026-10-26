import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { api } from "../../lib/api";

const NEW = "__new__";

// Repo names are fetched once per page load; the projects folder rarely changes.
let reposOnce: Promise<string[]> | null = null;
function repoNames(): Promise<string[]> {
  reposOnce ??= api.repos().then((rs) => rs.map((r) => r.name)).catch(() => []);
  return reposOnce;
}

/**
 * Pick an agent's project: one already in use, a git repo from the projects folder,
 * or a new name. `projects` are the ones other agents already use.
 */
export function ProjectSelect({ value, projects, onChange }: { value: string; projects: string[]; onChange: (v: string) => void }) {
  const [repos, setRepos] = useState<string[]>([]);
  const [adding, setAdding] = useState(false);
  const [before, setBefore] = useState(value);
  useEffect(() => {
    repoNames().then(setRepos);
  }, []);

  const inUse = [...new Set([...projects, value].filter(Boolean))].sort((a, b) => a.localeCompare(b));
  const taken = new Set(inUse.map((p) => p.toLowerCase()));
  const freeRepos = repos.filter((r) => !taken.has(r.toLowerCase()));

  if (adding) {
    return (
      <div className="flex gap-2">
        <input
          autoFocus
          className="input"
          placeholder="New project or tag"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), setAdding(false))}
          onBlur={() => setAdding(false)}
        />
        <button
          type="button"
          className="btn-ghost shrink-0 px-2"
          title="Back to the list"
          // Keep the input's blur from firing first and committing the half-typed name.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => (onChange(before), setAdding(false))}
        >
          <X className="size-3.5" />
        </button>
      </div>
    );
  }

  return (
    <select
      className="input"
      value={value}
      onChange={(e) => (e.target.value === NEW ? (setBefore(value), onChange(""), setAdding(true)) : onChange(e.target.value))}
    >
      <option value="">No project</option>
      {inUse.length > 0 && (
        <optgroup label="In use">
          {inUse.map((p) => <option key={p} value={p}>{p}</option>)}
        </optgroup>
      )}
      {freeRepos.length > 0 && (
        <optgroup label="Git repos">
          {freeRepos.map((r) => <option key={r} value={r}>{r}</option>)}
        </optgroup>
      )}
      <option value={NEW}>Add new…</option>
    </select>
  );
}
