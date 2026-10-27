import { useLayoutEffect, type RefObject } from "react";

/** Sizes a textarea to its visual lines, wrapped ones included, between `min` and `max` rows; past `max` it scrolls.
 * Runs on every change of `value`, so it also shrinks back when text is deleted or sent, and on window resize since wrapping follows the width. */
export function useAutoGrow(ref: RefObject<HTMLTextAreaElement | null>, value: string, min: number, max = 12) {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const fit = () => {
      const style = getComputedStyle(el);
      const line = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.5;
      const pad = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      // Collapse first so scrollHeight measures the text, not the current height.
      el.style.height = "auto";
      const lines = Math.round((el.scrollHeight - pad) / line);
      el.style.height = `${Math.min(max, Math.max(min, lines)) * line + pad}px`;
      el.style.overflowY = lines > max ? "auto" : "hidden";
    };
    fit();
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  }, [ref, value, min, max]);
}
