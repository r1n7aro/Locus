/** 3D model formats the preview viewer can parse. */
export type ModelFormat = "glb" | "fbx";

const MODEL_EXTENSIONS: Record<string, ModelFormat> = {
  glb: "glb",
  fbx: "fbx",
};

/** Format implied by a file name or path; `null` when it is not previewable. */
export function modelFormatFromPath(path: string): ModelFormat | null {
  const match = /\.([a-z0-9]+)$/i.exec(path.trim());
  return match ? MODEL_EXTENSIONS[match[1].toLowerCase()] ?? null : null;
}

/**
 * Format of the file content. Content wins over the extension because binary
 * blobs served by the backend (diff, asset cache) carry no reliable MIME type.
 */
export function detectModelFormat(buffer: ArrayBuffer, hint?: ModelFormat | null): ModelFormat | null {
  const head = new Uint8Array(buffer, 0, Math.min(buffer.byteLength, 64));
  const ascii = String.fromCharCode(...head);
  if (ascii.startsWith("glTF")) return "glb";
  if (ascii.startsWith("Kaydara FBX Binary") || /^;\s*FBX\b/.test(ascii)) return "fbx";
  return hint ?? null;
}

/** Decodes a `data:` URL payload without going through `fetch`. */
export function dataUrlToArrayBuffer(dataUrl: string): ArrayBuffer {
  const comma = dataUrl.indexOf(",");
  if (!dataUrl.startsWith("data:") || comma < 0) throw new Error("Invalid data URL");
  const payload = dataUrl.slice(comma + 1);
  const binary = /;base64$/i.test(dataUrl.slice(0, comma))
    ? atob(payload)
    : decodeURIComponent(payload);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes.buffer;
}
