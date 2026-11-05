// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MarkdownRenderer from "../components/MarkdownRenderer.vue";
import { closeImageLightbox } from "../components/ui/imageLightboxService";

let app: App | null = null;
let host: HTMLDivElement;

beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
});

afterEach(async () => {
  closeImageLightbox();
  await nextTick();
  app?.unmount();
  app = null;
  document.body.replaceChildren();
});

async function mountImage(props: Record<string, unknown> = {}): Promise<HTMLImageElement> {
  app = createApp({ render: () => h(MarkdownRenderer, { content: "![north map](https://example.com/north.png)", ...props }) });
  app.use(createPinia());
  app.mount(host);
  await vi.waitFor(() => expect(host.querySelector("img.md-image-preview")).not.toBeNull());
  const image = host.querySelector<HTMLImageElement>("img.md-image-preview")!;
  image.dataset.mdImageState = "ready";
  return image;
}

describe("MarkdownRenderer images", () => {
  it("floats document images in a lightbox when the host has no viewer of its own", async () => {
    const image = await mountImage();
    image.click();
    await nextTick();
    const lightbox = document.querySelector<HTMLElement>(".image-lightbox");
    expect(lightbox).not.toBeNull();
    expect(lightbox?.querySelector(".image-lightbox-image")?.getAttribute("src")).toBe(image.src);
    expect(lightbox?.querySelector(".image-lightbox-caption")?.textContent).toBe("north map");

    lightbox?.querySelector<HTMLButtonElement>(".image-lightbox-close")?.click();
    await vi.waitFor(() => expect(document.querySelector(".image-lightbox")).toBeNull());
  });

  it("hands images to a host viewer such as Chat instead", async () => {
    const onOpenImage = vi.fn();
    const image = await mountImage({ onOpenImage });
    image.click();
    await nextTick();
    expect(onOpenImage).toHaveBeenCalledWith(image.src);
    expect(document.querySelector(".image-lightbox")).toBeNull();
  });
});
