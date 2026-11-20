/** Whether an Escape press closes a dialog: not when something already handled it (a field's menu, say). */
export function closesOnEscape(e: { key: string; defaultPrevented: boolean }): boolean {
  return e.key === "Escape" && !e.defaultPrevented;
}

/** Whether a key comes from a field, whose own handlers (a tag or @-mention menu) may take Esc before the dialog does. */
export function fromField(target: EventTarget | null): boolean {
  const el = target as { tagName?: string; isContentEditable?: boolean } | null;
  return !!el?.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el?.tagName ?? "");
}
