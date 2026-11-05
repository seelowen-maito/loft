import { useEffect, useState } from "react";
import { Plus } from "lucide-react";
import { api, type Group } from "../lib/api";
import { cx } from "../lib/format";
import { ProjectsField } from "./agents/ProjectsField";

/** Settings → Groups: each group's projects, its main project (the star: where its specs go) and an optional folder whose repos are suggested. */
export function GroupSettings() {
  const [groups, setGroups] = useState<Group[]>([]);
  const [adding, setAdding] = useState("");
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const load = () => {
    api.groups().then(setGroups).catch((e) => setMsg({ ok: false, text: (e as Error).message }));
  };
  useEffect(load, []);

  const add = async () => {
    const name = adding.trim();
    if (!name) return;
    setMsg(null);
    try {
      await api.saveGroup(name, {});
      setAdding("");
      load();
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    }
  };

  return (
    <section className="panel space-y-4 p-5">
      <div>
        <h2 className="text-[15px] font-medium tracking-tight">Groups</h2>
        <p className="mt-1 text-[12px] text-fg-3">A group's projects and its main project (the star), where its specs go. With a folder, its git repos are suggested as projects.</p>
      </div>
      {groups.map((g) => (
        <GroupRow key={g.name} group={g} onSaved={(n) => setGroups((gs) => gs.map((x) => (x.name === n.name ? n : x)))} />
      ))}
      {groups.length === 0 && <p className="text-[12.5px] text-fg-3">No groups yet.</p>}
      <div className="flex gap-2">
        <input className="input" value={adding} placeholder="New group, e.g. Acme" maxLength={100} onChange={(e) => setAdding(e.target.value)} onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), add())} aria-label="New group" />
        <button type="button" className="btn shrink-0" onClick={add} disabled={!adding.trim()} title="Add the group">
          <Plus className="size-3.5" />
        </button>
      </div>
      {msg && <p className={cx("text-[12px]", msg.ok ? "text-fg-2" : "text-bad")}>{msg.text}</p>}
    </section>
  );
}

/** Main project first, as the project chips show it. */
const mainFirst = (g: Group) => (g.main_project ? [g.main_project, ...g.projects.filter((p) => p !== g.main_project)] : g.projects);

function GroupRow({ group, onSaved }: { group: Group; onSaved: (g: Group) => void }) {
  const [projects, setProjects] = useState(mainFirst(group));
  const [folder, setFolder] = useState(group.folder);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const dirty = folder !== group.folder || projects.join("\n") !== mainFirst(group).join("\n");

  const save = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const g = await api.saveGroup(group.name, { folder: folder.trim(), projects, main_project: projects[0] ?? "" });
      setProjects(mainFirst(g));
      setFolder(g.folder);
      onSaved(g);
      setMsg({ ok: true, text: "Saved." });
    } catch (e) {
      setMsg({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(false);
    }
  };

  const suggestions = group.suggestions.filter((s) => !projects.includes(s));
  return (
    <div className="space-y-2.5 rounded-xl border border-line p-3.5">
      <div className="flex items-center gap-2">
        <span className="font-medium">{group.name}</span>
        <span className="ml-auto font-mono text-[11px] text-fg-3">{projects.length} project{projects.length === 1 ? "" : "s"}</span>
      </div>
      <label className="block space-y-1">
        <span className="text-[12px] text-fg-2">Folder (optional)</span>
        <input className="input" value={folder} placeholder="A folder of repos in the projects folder, e.g. dineout" onChange={(e) => setFolder(e.target.value)} aria-label={`${group.name} folder`} />
      </label>
      <div className="space-y-1">
        <span className="text-[12px] text-fg-2">Projects</span>
        <ProjectsField value={projects} projects={[]} onChange={setProjects} />
      </div>
      {suggestions.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[11.5px] text-fg-3">In the folder:</span>
          {suggestions.map((s) => (
            <button key={s} type="button" onClick={() => setProjects([...projects, s])} title={`Add ${s}`} className="inline-flex items-center gap-1 rounded-full border border-dashed border-line py-0.5 pl-1.5 pr-2 font-mono text-[11.5px] text-fg-2 hover:text-fg">
              <Plus className="size-3" />
              {s}
            </button>
          ))}
        </div>
      )}
      <div className="flex items-center gap-3">
        <button type="button" className="btn" onClick={save} disabled={!dirty || busy}>
          Save
        </button>
        {msg && <span className={cx("text-[12px]", msg.ok ? "text-fg-2" : "text-bad")}>{msg.text}</span>}
      </div>
    </div>
  );
}
