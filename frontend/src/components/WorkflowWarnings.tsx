import { TriangleAlert } from "lucide-react";
import type { WorkflowCheck } from "../lib/api";
import { Callout } from "./Callout";

/** Text with `code` spans, as the server words a warning. */
function Ticks({ text }: { text: string }) {
  return <>{text.split("`").map((part, i) => (i % 2 ? <code key={i} className="font-mono text-[11.5px] text-fg">{part}</code> : part))}</>;
}

/** The workflow check of a private repo, on the end-of-planning card and the Delivery card: what its workflows spend
 *  on the draft spec PR for nothing, and the Actions minutes each ticket will spend. A warning only: it blocks nothing. */
export function WorkflowWarnings({ check, className }: { check: WorkflowCheck; className?: string }) {
  return (
    <Callout tone="warn" icon={<TriangleAlert />} className={className}>
      <div data-workflow-warnings>
        <div className="text-fg">Its workflows spend more Actions minutes than they need to on this private repo.</div>
        <ul className="mt-1 list-disc space-y-1 pl-4">
          {check.warnings.map((w) => (
            <li key={`${w.workflow}:${w.kind}`}>
              <Ticks text={w.text} />
            </li>
          ))}
        </ul>
        <div className="mt-1.5 text-fg-3">{check.text} Loft changes no workflow, and the Delivery runs either way.</div>
      </div>
    </Callout>
  );
}
