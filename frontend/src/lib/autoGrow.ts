import { useLayoutEffect, type RefObject } from "react";

/** How many rows a textarea shows for text measuring `contentHeight` px of `lineHeight` px lines, between `min` and `max`;
 * `scroll` is on only when the text runs past `max`. Empty text takes `min`, since its scrollHeight measures the placeholder. */
export function fitRows(contentHeight: number, lineHeight: number, empty: boolean, min: number, max = 12) {
  const lines = empty ? min : Math.round(contentHeight / lineHeight);
  return { rows: Math.min(max, Math.max(min, lines)), scroll: lines > max };
}

/** Sizes a textarea to its visual lines, wrapped ones included, between `min` and `max` rows; past `max` it scrolls.
 * Runs on every change of `value`, so it also shrinks back when text is deleted or sent, and when the textarea's width changes, since wrapping follows it. */
export function useAutoGrow(ref: RefObject<HTMLTextAreaElement | null>, value: string, min: number, max = 12) {
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    let width = el.clientWidth;
    const fit = () => {
      const style = getComputedStyle(el);
      const line = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.5;
      const pad = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      // Collapse first so scrollHeight measures the text, not the current height.
      el.style.height = "auto";
      const { rows, scroll } = fitRows(el.scrollHeight - pad, line, !el.value, min, max);
      el.style.height = `${rows * line + pad}px`;
      el.style.overflowY = scroll ? "auto" : "hidden";
    };
    fit();
    // Our own height changes also reach the observer: refit only when the width moved.
    const observer = new ResizeObserver(() => {
      if (el.clientWidth === width) return;
      width = el.clientWidth;
      fit();
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, value, min, max]);
}
