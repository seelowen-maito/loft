import { useState } from "react";
import { AlertCircle, Loader2, X } from "lucide-react";
import type { Attachment } from "../lib/useDraft";
import { cx } from "../lib/format";

/** `onMissing` fires when an uploaded image's file can't be loaded, e.g. a Draft's after the chat history was cleared. */
export function AttachmentTile({ a, onRemove, onMissing }: { a: Attachment; onRemove: () => void; onMissing: () => void }) {
  const [loaded, setLoaded] = useState(false);
  const busy = !a.file && !a.error;
  return (
    <div
      className={cx("group relative size-24 overflow-hidden rounded-xl border bg-ink-950", a.error ? "border-bad/50" : "border-line")}
      title={a.error}
    >
      <img
        src={a.preview}
        alt="Pasted image"
        onLoad={() => setLoaded(true)}
        onError={() => a.file && onMissing()}
        className={cx("size-full object-contain transition duration-300", loaded ? "opacity-100" : "opacity-0", busy && "opacity-40 blur-[1px]")}
      />
      {busy && (
        <div className="absolute inset-0 grid place-items-center" role="status" aria-label="Uploading image">
          <Loader2 className="size-5 animate-spin text-fg-2" />
        </div>
      )}
      {a.error && (
        <div className="absolute inset-0 grid place-items-center bg-bad/10">
          <AlertCircle className="size-5 text-bad" />
        </div>
      )}
      <button
        onClick={onRemove}
        aria-label="Remove image"
        className="absolute right-1 top-1 grid size-5 place-items-center rounded-full bg-ink-950/80 text-fg-2 opacity-0 transition hover:text-fg focus-visible:opacity-100 group-hover:opacity-100 [@media(hover:none)]:opacity-100"
      >
        <X className="size-3" />
      </button>
    </div>
  );
}
