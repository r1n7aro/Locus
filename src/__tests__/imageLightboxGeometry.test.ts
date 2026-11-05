import { describe, expect, it } from "vitest";
import {
  IMAGE_LIGHTBOX_PADDING,
  clampImageView,
  fitImageScale,
  imageClickZoomScale,
  imageScaleBounds,
  zoomImageView,
} from "../components/ui/imageLightboxGeometry";

const stage = { width: 1000 + IMAGE_LIGHTBOX_PADDING.x * 2, height: 800 + IMAGE_LIGHTBOX_PADDING.y * 2 };

describe("image lightbox geometry", () => {
  it("fits large images inside the padded stage without enlarging small ones", () => {
    expect(fitImageScale({ width: 2000, height: 3200 }, stage)).toBe(0.25);
    expect(fitImageScale({ width: 400, height: 300 }, stage)).toBe(1);
    expect(imageScaleBounds({ width: 2000, height: 3200 }, stage)).toEqual({ min: 0.25, max: 4 });
    // Small images may still be magnified to fill twice the screen.
    expect(imageScaleBounds({ width: 50, height: 40 }, stage).max).toBe(40);
  });

  it("zooms a click to actual pixels, or twice the fitted size for small images", () => {
    expect(imageClickZoomScale({ width: 2000, height: 3200 }, stage)).toBe(1);
    expect(imageClickZoomScale({ width: 400, height: 300 }, stage)).toBe(2);
  });

  it("keeps the image point under the cursor fixed while zooming", () => {
    const natural = { width: 2000, height: 3200 };
    const anchor = { x: 200, y: -100 };
    const before = { scale: 0.5, x: 0, y: 0 };
    const after = zoomImageView(before, 1, anchor, natural, stage);
    const pointBefore = { x: (anchor.x - before.x) / before.scale, y: (anchor.y - before.y) / before.scale };
    const pointAfter = { x: (anchor.x - after.x) / after.scale, y: (anchor.y - after.y) / after.scale };
    expect(after.scale).toBe(1);
    expect(pointAfter).toEqual(pointBefore);
  });

  it("clamps scale and pans so a zoomed image never leaves an empty edge", () => {
    const natural = { width: 2000, height: 3200 };
    expect(zoomImageView({ scale: 1, x: 0, y: 0 }, 100, { x: 0, y: 0 }, natural, stage).scale).toBe(4);
    expect(clampImageView({ scale: 1, x: 5000, y: -5000 }, natural, stage)).toEqual({
      scale: 1,
      x: (2000 - stage.width) / 2,
      y: -(3200 - stage.height) / 2,
    });
    // Axes that fit in the stage stay centered.
    expect(clampImageView({ scale: 0.25, x: 40, y: 40 }, natural, stage)).toEqual({ scale: 0.25, x: 0, y: 0 });
  });
});
