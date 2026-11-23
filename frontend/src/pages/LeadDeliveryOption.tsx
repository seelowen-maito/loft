import { useEffect, useState } from "react";
import { api, type LeadDeliveryOptions, type LeadDeliveryView } from "../lib/api";
import { SettingsCard } from "../components/SettingsCard";
import { ErrorLine } from "../components/Callout";
import { errorText } from "../lib/useAction";
import { fmtDateTime } from "../lib/time";
import { Toggle } from "./Settings";

const LIMITS: { key: "resumes_per_hour" | "resumes_per_day" | "running_deliveries"; label: string }[] = [
  { key: "resumes_per_hour", label: "Resumes per hour" },
  { key: "resumes_per_day", label: "Resumes per day" },
  { key: "running_deliveries", label: "Deliveries running at once" },
];

/** What a log line says changed, e.g. "enabled: off → on, resumes per day: 6 → 10". */
export function changeText(before: LeadDeliveryOptions, after: LeadDeliveryOptions): string {
  const parts: string[] = [];
  if (before.enabled !== after.enabled) parts.push(`${after.enabled ? "turned on" : "turned off"}`);
  for (const l of LIMITS) if (before[l.key] !== after[l.key]) parts.push(`${l.label.toLowerCase()}: ${before[l.key]} → ${after[l.key]}`);
  return parts.join(", ");
}

/** Per project: may the Lead start and resume Deliveries, and how often. Saves each change at once. */
export function LeadDeliveryOption() {
  const [projects, setProjects] = useState<string[]>([]);
  const [project, setProject] = useState("");
  const [view, setView] = useState<LeadDeliveryView | null>(null);
  const [err, setErr] = useState("");

  useEffect(() => {
    api.repos().then((r) => {
      const names = (r ?? []).map((x) => x.name);
      setProjects(names);
      setProject((p) => p || names[0] || "");
    }).catch((e) => setErr(errorText(e)));
  }, []);
  useEffect(() => {
    if (!project) return;
    setView(null);
    api.leadDelivery(project).then(setView).catch((e) => setErr(errorText(e)));
  }, [project]);

  const save = async (change: Partial<LeadDeliveryOptions>) => {
    setErr("");
    try {
      setView(await api.saveLeadDelivery(project, change));
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <SettingsCard title="Lead and Deliveries">
      <label className="block space-y-1.5">
        <span className="eyebrow">Project</span>
        <select className="input" value={project} onChange={(e) => setProject(e.target.value)}>
          {projects.map((p) => <option key={p} value={p}>{p}</option>)}
        </select>
      </label>
      {view && (
        <>
          <Toggle
            on={view.options.enabled}
            onChange={(v) => save({ enabled: v })}
            label="The Lead can start and resume Deliveries"
            hint="Off by default. On: the team's Lead can start a published spec's Delivery and resume one stalled for a transient reason (usage limit, API error), within the limits below. You still decide what is in Needs you, and merge."
          />
          <div className="grid gap-3 sm:grid-cols-3">
            {LIMITS.map((l) => (
              <label key={l.key} className="block space-y-1.5">
                <span className="eyebrow">{l.label}</span>
                <input
                  className="input"
                  type="number"
                  min={1}
                  max={view.ceilings[l.key]}
                  value={view.options[l.key]}
                  onChange={(e) => save({ [l.key]: Number(e.target.value) })}
                />
                <span className="block text-[11.5px] text-fg-3">1 to {view.ceilings[l.key]}</span>
              </label>
            ))}
          </div>
          <div className="space-y-1">
            <span className="eyebrow">Changes</span>
            {view.log.length === 0 ? (
              <p className="text-[12.5px] text-fg-3">No changes yet.</p>
            ) : (
              <ul className="space-y-0.5 text-[12.5px] text-fg-2">
                {view.log.map((l, i) => (
                  <li key={i}>
                    <span className="font-mono text-[11px] text-fg-3">{fmtDateTime(l.at)}</span> · {l.by}: {changeText(l.before, l.after)}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </>
      )}
      <ErrorLine error={err} />
    </SettingsCard>
  );
}
