/** Whether an Escape press closes a dialog: not when a field has it (Esc leaves the field's own edit alone) or something already handled it. */
export function closesOnEscape(e: { key: string; defaultPrevented: boolean; target: EventTarget | null }): boolean {
  if (e.key !== "Escape" || e.defaultPrevented) return false;
  const el = e.target as { tagName?: string; isContentEditable?: boolean } | null;
  return !(el?.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el?.tagName ?? ""));
}
