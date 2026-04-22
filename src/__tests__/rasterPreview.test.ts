import { describe, expect, it } from "vitest";
import {
  buildTransformedPixels,
  defaultRasterAlphaMode,
  getRasterMetaAlphaState,
  resolveAlphaAsTransparency,
} from "../components/diff/rasterPreview";

describe("rasterPreview", () => {
  it("maps Unity meta alpha transparency into the default visible mode", () => {
    expect(defaultRasterAlphaMode({ alphaIsTransparency: true })).toBe("transparent");
    expect(defaultRasterAlphaMode({ alphaIsTransparency: false })).toBe("opaque");
    expect(defaultRasterAlphaMode(undefined)).toBe("transparent");
    expect(resolveAlphaAsTransparency("transparent")).toBe(true);
    expect(resolveAlphaAsTransparency("opaque")).toBe(false);
  });

  it("renders opaque color view by forcing alpha to 255", () => {
    const source = new Uint8ClampedArray([10, 20, 30, 40]);
    expect(Array.from(buildTransformedPixels(source, "color", false))).toEqual([
      10, 20, 30, 255,
    ]);
  });

  it("renders single channels as grayscale", () => {
    const source = new Uint8ClampedArray([10, 20, 30, 40]);
    expect(Array.from(buildTransformedPixels(source, "r", true))).toEqual([
      10, 10, 10, 255,
    ]);
    expect(Array.from(buildTransformedPixels(source, "a", true))).toEqual([
      40, 40, 40, 255,
    ]);
  });

  it("reports meta alpha state for toolbar badges", () => {
    expect(getRasterMetaAlphaState({ alphaIsTransparency: true })).toBe("enabled");
    expect(getRasterMetaAlphaState({ alphaIsTransparency: false })).toBe("disabled");
    expect(getRasterMetaAlphaState(undefined)).toBe("unknown");
  });
});
