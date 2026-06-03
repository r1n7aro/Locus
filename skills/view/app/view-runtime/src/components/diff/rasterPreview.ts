import type { UnityTexturePreviewMeta } from "../../types";

export type RasterAlphaMode = "transparent" | "opaque";
export type RasterChannelMode = "color" | "r" | "g" | "b" | "a";
export type RasterMetaAlphaState = "enabled" | "disabled" | "unknown";

export function defaultRasterAlphaMode(
  meta: UnityTexturePreviewMeta | undefined,
): RasterAlphaMode {
  return meta?.alphaIsTransparency === false ? "opaque" : "transparent";
}

export function resolveAlphaAsTransparency(alphaMode: RasterAlphaMode): boolean {
  return alphaMode === "transparent";
}

export function getRasterMetaAlphaState(
  meta: UnityTexturePreviewMeta | undefined,
): RasterMetaAlphaState {
  if (meta?.alphaIsTransparency === true) return "enabled";
  if (meta?.alphaIsTransparency === false) return "disabled";
  return "unknown";
}

export function buildTransformedPixels(
  source: ArrayLike<number>,
  channelMode: RasterChannelMode,
  alphaAsTransparency: boolean,
): Uint8ClampedArray {
  const out = new Uint8ClampedArray(source.length);

  for (let i = 0; i < source.length; i += 4) {
    if (channelMode === "color") {
      out[i] = source[i] ?? 0;
      out[i + 1] = source[i + 1] ?? 0;
      out[i + 2] = source[i + 2] ?? 0;
      out[i + 3] = alphaAsTransparency ? (source[i + 3] ?? 255) : 255;
      continue;
    }

    const offset = channelMode === "r"
      ? 0
      : channelMode === "g"
        ? 1
        : channelMode === "b"
          ? 2
          : 3;
    const value = source[i + offset] ?? 0;
    out[i] = value;
    out[i + 1] = value;
    out[i + 2] = value;
    out[i + 3] = 255;
  }

  return out;
}

export function drawRasterToCanvas(
  canvas: HTMLCanvasElement,
  source: ImageData,
  channelMode: RasterChannelMode,
  alphaAsTransparency: boolean,
) {
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    throw new Error("2D canvas is unavailable");
  }

  canvas.width = source.width;
  canvas.height = source.height;

  if (channelMode === "color" && alphaAsTransparency) {
    ctx.putImageData(source, 0, 0);
    return;
  }

  const pixels = buildTransformedPixels(source.data, channelMode, alphaAsTransparency);
  ctx.putImageData(new ImageData(pixels, source.width, source.height), 0, 0);
}
