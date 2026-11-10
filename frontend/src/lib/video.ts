// Videos in the chat: which files can be attached, and how they read once sent.

/** Videos go up as they are; the backend sends the agent frames of them. */
export const VIDEO_TYPES = ["video/mp4", "video/quicktime", "video/webm"];
export const MAX_VIDEO_BYTES = 200_000_000;

/** An upload's stored name (`<uuid>.<ext>`) is a video's. */
export const isVideo = (file: string) => /\.(mp4|mov|webm)$/.test(file);

/** A video's length as the chat shows it: `0:07`, `12:34`. */
export function fmtLength(secs: number): string {
  if (!Number.isFinite(secs)) return "";
  const s = Math.round(secs);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Which of `all` can be attached: images, and with `videos` MP4, MOV and WebM up to MAX_VIDEO_BYTES. */
export function attachable<F extends { type: string; size: number }>(all: F[], videos: boolean) {
  const video = (f: { type: string }) => videos && VIDEO_TYPES.includes(f.type);
  const ok = all.filter((f) => f.type.startsWith("image/") || (video(f) && f.size <= MAX_VIDEO_BYTES));
  const why = ok.length === all.length ? null
    : all.some((f) => video(f) && f.size > MAX_VIDEO_BYTES) ? "Videos can be up to 200 MB."
    : videos ? "Only images and MP4, MOV or WebM videos can be attached." : "Only images can be attached.";
  return { ok, why };
}
