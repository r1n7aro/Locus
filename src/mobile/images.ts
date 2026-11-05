import type { ImageAttachment } from "../types";

/** Long edge of an attached photo. Phone photos are 3–12 MB; at this size
 * they stay well inside model image limits and the 16 MB link frame. */
const MAX_EDGE = 2048;
/** Small images in a format every model accepts are sent unchanged. */
const KEEP_ORIGINAL_BYTES = 1_500_000;
const JPEG_QUALITY = 0.85;
const PASSTHROUGH_TYPES = new Set(["image/png", "image/jpeg", "image/webp", "image/gif"]);

function readBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = String(reader.result ?? "");
      const comma = dataUrl.indexOf(",");
      if (comma < 0) reject(new Error("Unreadable image"));
      else resolve(dataUrl.slice(comma + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error("Unreadable image"));
    reader.readAsDataURL(blob);
  });
}

/** Reads a picked image, scaling large photos down and re-encoding them as
 * JPEG (orientation from EXIF is applied by the decoder). */
export async function prepareImageAttachment(file: File): Promise<ImageAttachment> {
  const bitmap = await createImageBitmap(file);
  try {
    const scale = Math.min(1, MAX_EDGE / Math.max(bitmap.width, bitmap.height));
    if (scale === 1 && file.size <= KEEP_ORIGINAL_BYTES && PASSTHROUGH_TYPES.has(file.type)) {
      return { data: await readBase64(file), mimeType: file.type };
    }
    const width = Math.max(1, Math.round(bitmap.width * scale));
    const height = Math.max(1, Math.round(bitmap.height * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas unavailable");
    // JPEG has no transparency; screenshots with alpha get a white page.
    context.fillStyle = "#fff";
    context.fillRect(0, 0, width, height);
    context.drawImage(bitmap, 0, 0, width, height);
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", JPEG_QUALITY));
    if (!blob) throw new Error("Image encoding failed");
    return { data: await readBase64(blob), mimeType: "image/jpeg" };
  } finally {
    bitmap.close();
  }
}
