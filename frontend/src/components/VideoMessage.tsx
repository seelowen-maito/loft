import { useState } from "react";
import { fmtLength } from "../lib/video";

/** A video sent in the chat: an inline player, with its length under it once known. */
export function VideoMessage({ src }: { src: string }) {
  const [length, setLength] = useState<number | null>(null);
  return (
    <figure className="flex max-w-full flex-col items-end gap-1">
      <video
        src={src}
        controls
        playsInline
        preload="metadata"
        onLoadedMetadata={(e) => setLength(e.currentTarget.duration)}
        className="max-h-72 max-w-full rounded-xl border border-line bg-ink-950"
      />
      {length !== null && <figcaption className="font-mono text-[10.5px] text-fg-3">Video · {fmtLength(length)}</figcaption>}
    </figure>
  );
}
