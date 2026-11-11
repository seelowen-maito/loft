import { useState } from "react";
import { fmtLength } from "../lib/video";

/** A video sent in the chat: an inline player, with its length under it once known. */
export function VideoMessage({ src }: { src: string }) {
  const [length, setLength] = useState<number | null>(null);
  const [failed, setFailed] = useState(false);
  if (failed)
    return (
      <div className="max-w-full rounded-xl border border-line px-3 py-2 text-[12.5px] text-fg-3">
        This browser can't play this video.{" "}
        <a href={src} download className="text-fg-2 underline">Download it</a>
      </div>
    );
  return (
    <figure className="flex max-w-full flex-col items-end gap-1">
      <video
        src={src}
        controls
        playsInline
        preload="metadata"
        onError={() => setFailed(true)}
        onLoadedMetadata={(e) => setLength(e.currentTarget.duration)}
        className="max-h-72 max-w-full rounded-xl border border-line bg-ink-950"
      />
      {length !== null && <figcaption className="font-mono text-[10.5px] text-fg-3">Video · {fmtLength(length)}</figcaption>}
    </figure>
  );
}
