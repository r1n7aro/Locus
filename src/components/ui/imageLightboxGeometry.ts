export interface ImageLightboxRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface ImageLightboxImage {
  src: string;
  alt?: string;
  /** Viewport rect of the inline image; the enlarged image grows out of it. */
  originRect?: ImageLightboxRect | null;
}

export interface ImageLightboxSize {
  width: number;
  height: number;
}

/** Scale is relative to the natural image size; x/y offset the image center from the stage center. */
export interface ImageLightboxView {
  scale: number;
  x: number;
  y: number;
}

/** Space kept free around a fitted image for the toolbar and caption. */
export const IMAGE_LIGHTBOX_PADDING = { x: 32, y: 56 } as const;
const MIN_MAX_SCALE = 4;

function containScale(natural: ImageLightboxSize, stage: ImageLightboxSize): number {
  if (natural.width <= 0 || natural.height <= 0) return 1;
  const width = Math.max(1, stage.width - IMAGE_LIGHTBOX_PADDING.x * 2);
  const height = Math.max(1, stage.height - IMAGE_LIGHTBOX_PADDING.y * 2);
  return Math.min(width / natural.width, height / natural.height);
}

/** Shows the whole image, never enlarging it past its natural size. */
export function fitImageScale(natural: ImageLightboxSize, stage: ImageLightboxSize): number {
  return Math.min(1, containScale(natural, stage));
}

export function imageScaleBounds(natural: ImageLightboxSize, stage: ImageLightboxSize): { min: number; max: number } {
  const min = fitImageScale(natural, stage);
  return { min, max: Math.max(min, MIN_MAX_SCALE, containScale(natural, stage) * 2) };
}

/** Click-to-zoom target: actual pixels, or twice the fitted size for small images. */
export function imageClickZoomScale(natural: ImageLightboxSize, stage: ImageLightboxSize): number {
  const { min, max } = imageScaleBounds(natural, stage);
  return Math.min(max, Math.max(1, min * 2));
}

/** Keeps a zoomed image covering the stage and centers axes that fit inside it. */
export function clampImageView(
  view: ImageLightboxView,
  natural: ImageLightboxSize,
  stage: ImageLightboxSize,
): ImageLightboxView {
  const { min, max } = imageScaleBounds(natural, stage);
  const scale = Math.min(max, Math.max(min, view.scale));
  const maxX = Math.max(0, (natural.width * scale - stage.width) / 2);
  const maxY = Math.max(0, (natural.height * scale - stage.height) / 2);
  return {
    scale,
    x: Math.min(maxX, Math.max(-maxX, view.x)),
    y: Math.min(maxY, Math.max(-maxY, view.y)),
  };
}

/** Zooms while keeping the image point under `anchor` (relative to the stage center) in place. */
export function zoomImageView(
  view: ImageLightboxView,
  nextScale: number,
  anchor: { x: number; y: number },
  natural: ImageLightboxSize,
  stage: ImageLightboxSize,
): ImageLightboxView {
  const { min, max } = imageScaleBounds(natural, stage);
  const scale = Math.min(max, Math.max(min, nextScale));
  const ratio = view.scale > 0 ? scale / view.scale : 1;
  return clampImageView({
    scale,
    x: anchor.x - (anchor.x - view.x) * ratio,
    y: anchor.y - (anchor.y - view.y) * ratio,
  }, natural, stage);
}
