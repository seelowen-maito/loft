import { useState, type Dispatch, type DragEvent, type SetStateAction } from "react";
import { api } from "./api";
import type { Attachment } from "./useDraft";

/** The API's limits: 3.75 MB raw (5 MB base64), and it scales anything bigger than 1568 px down anyway. */
const MAX_IMAGE_BYTES = 3_750_000;
const MAX_IMAGE_EDGE = 1568;
export const MAX_IMAGES = 10;
const SENDABLE = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/** The image as-is when the API takes it, else redrawn to fit: PNG keeps screenshots sharp, JPEG if that's still too big. */
async function prepareImage(f: File): Promise<Blob> {
  const bmp = await createImageBitmap(f).catch(() => {
    throw new Error("Couldn't read this image");
  });
  const scale = Math.min(1, MAX_IMAGE_EDGE / Math.max(bmp.width, bmp.height));
  if (scale === 1 && f.size <= MAX_IMAGE_BYTES && SENDABLE.includes(f.type)) return f;
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(bmp.width * scale);
  canvas.height = Math.round(bmp.height * scale);
  canvas.getContext("2d")!.drawImage(bmp, 0, 0, canvas.width, canvas.height);
  const encode = (type: string, q?: number) => new Promise<Blob | null>((r) => canvas.toBlob(r, type, q));
  const png = await encode("image/png");
  if (png && png.size <= MAX_IMAGE_BYTES) return png;
  const jpeg = await encode("image/jpeg", 0.85);
  if (jpeg && jpeg.size <= MAX_IMAGE_BYTES) return jpeg;
  throw new Error("Image is too large");
}

/** Starts uploading the images among `all` for `agent`, each shown at once as an attachment, up to MAX_IMAGES with those `have` already.
 * Returns what to tell the user about the files left out, or null. */
export function attachImages(agent: string, all: File[], have: number, setAttachments: Dispatch<SetStateAction<Attachment[]>>, per = "message"): string | null {
  const images = all.filter((f) => f.type.startsWith("image/"));
  const files = images.slice(0, MAX_IMAGES - have);
  for (const f of files) {
    const key = crypto.randomUUID();
    setAttachments((a) => [...a, { key, preview: URL.createObjectURL(f) }]);
    const update = (u: Partial<Attachment>) => setAttachments((a) => a.map((x) => (x.key === key ? { ...x, ...u } : x)));
    prepareImage(f)
      .then((blob) => api.upload(agent, blob))
      .then((r) => update({ file: r.file }), (e) => update({ error: (e as Error).message }));
  }
  return files.length < images.length ? `Up to ${MAX_IMAGES} images per ${per}.`
    : images.length < all.length ? "Only images can be attached." : null;
}

/** The images on a paste, taken instead of the text: copying an image from a page also puts its address on the clipboard. */
export function pastedImages(e: { clipboardData: DataTransfer; preventDefault: () => void }): File[] {
  const files = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"));
  if (files.length > 0) e.preventDefault();
  return files;
}

/** Drag and drop of files onto an element: `dragging` while files are over it, `onFiles` with the dropped ones.
 * With `own`, the drag stops here, so an outer drop zone neither shows its hint nor takes the files. */
export function useFileDrop(onFiles: (files: File[]) => void, own = false) {
  // dragenter/dragleave fire for every child crossed; count them to know when the drag really left.
  const [depth, setDepth] = useState(0);
  const files = (e: DragEvent) => e.dataTransfer.types.includes("Files");
  const stop = (e: DragEvent) => own && e.stopPropagation();
  return {
    dragging: depth > 0,
    reset: () => setDepth(0),
    handlers: {
      onDragEnter: (e: DragEvent) => files(e) && (stop(e), setDepth((d) => d + 1)),
      onDragLeave: (e: DragEvent) => files(e) && (stop(e), setDepth((d) => Math.max(0, d - 1))),
      onDragOver: (e: DragEvent) => files(e) && (stop(e), e.preventDefault()),
      onDrop: (e: DragEvent) => {
        if (!files(e)) return;
        stop(e);
        e.preventDefault();
        setDepth(0);
        onFiles([...e.dataTransfer.files]);
      },
    },
  };
}
