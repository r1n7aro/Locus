import { describe, expect, it, vi } from "vitest";
import {
  DEFERRED_SESSION_MESSAGE_IMAGE_PREFIX,
  deferredSessionMessageImageId,
  resolveToolResultImages,
} from "../composables/toolResultImages";

describe("tool result image hydration", () => {
  it("recognizes deferred session message image markers", () => {
    expect(deferredSessionMessageImageId({
      data: `${DEFERRED_SESSION_MESSAGE_IMAGE_PREFIX}message-1`,
      mimeType: "image/png",
    })).toBe("message-1");
    expect(deferredSessionMessageImageId({ data: "base64", mimeType: "image/png" })).toBeNull();
  });

  it("loads each deferred message once and keeps inline images", async () => {
    const inline = { data: "inline", mimeType: "image/webp" };
    const loaded = [
      { data: "first", mimeType: "image/png" },
      { data: "second", mimeType: "image/png" },
    ];
    const loadMessageImages = vi.fn().mockResolvedValue(loaded);

    await expect(resolveToolResultImages([
      inline,
      { data: `${DEFERRED_SESSION_MESSAGE_IMAGE_PREFIX}message-1`, mimeType: "image/png" },
      { data: `${DEFERRED_SESSION_MESSAGE_IMAGE_PREFIX}message-1`, mimeType: "image/png" },
    ], loadMessageImages)).resolves.toEqual([inline, ...loaded]);
    expect(loadMessageImages).toHaveBeenCalledOnce();
    expect(loadMessageImages).toHaveBeenCalledWith("message-1");
  });
});
