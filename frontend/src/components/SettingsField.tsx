import { useId, type ReactNode } from "react";
import { HelpHint } from "./HelpHint";

/** A labelled setting. `children` gets the id for its input, so the label names it. `hint` is the short line under the
 * field; `help` is the rest, behind a (?) next to the label (outside the <label>, which names only the input). */
export function SettingsField({ label, hint, help, restart, children }: { label: string; hint?: string; help?: string; restart?: boolean; children: (id: string) => ReactNode }) {
  const id = useId();
  return (
    <div className="space-y-1.5">
      <div className="relative flex items-center gap-2">
        <label htmlFor={id} className="eyebrow">{label}</label>
        {restart && <span className="chip text-[10px]">restart</span>}
        {help && <HelpHint label={label}>{help}</HelpHint>}
      </div>
      {children(id)}
      {hint && <span className="block text-[11.5px] text-fg-3">{hint}</span>}
    </div>
  );
}
