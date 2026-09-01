// @vitest-environment jsdom

import { createApp } from "vue";
import { afterEach, describe, expect, it } from "vitest";
import QueuedFollowUpImages from "../components/chat/QueuedFollowUpImages.vue";

let app: ReturnType<typeof createApp> | null = null;

afterEach(() => {
  app?.unmount();
  app = null;
});

describe("QueuedFollowUpImages", () => {
  it("renders queued image payloads as compact previews", () => {
    const host = document.createElement("div");
    app = createApp(QueuedFollowUpImages, {
      images: [
        { data: "first", mimeType: "image/png" },
        { data: "second", mimeType: "image/jpeg" },
      ],
    });
    app.mount(host);

    const previews = host.querySelectorAll<HTMLImageElement>(".queued-follow-up-image");
    expect(previews).toHaveLength(2);
    expect(previews[0]?.src).toBe("data:image/png;base64,first");
    expect(previews[1]?.src).toBe("data:image/jpeg;base64,second");
  });

  it("keeps the queued bar compact when several images are pending", () => {
    const host = document.createElement("div");
    app = createApp(QueuedFollowUpImages, {
      images: Array.from({ length: 5 }, (_, index) => ({
        data: `image-${index}`,
        mimeType: "image/png",
      })),
    });
    app.mount(host);

    expect(host.querySelectorAll(".queued-follow-up-image")).toHaveLength(3);
    expect(host.querySelector(".queued-follow-up-image-count")?.textContent?.trim()).toBe("+2");
  });
});
