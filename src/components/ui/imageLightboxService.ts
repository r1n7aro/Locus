import { defineComponent, h, render, shallowRef } from "vue";
import ImageLightbox from "./ImageLightbox.vue";
import type { ImageLightboxImage } from "./imageLightboxGeometry";

export type { ImageLightboxImage, ImageLightboxRect } from "./imageLightboxGeometry";

const currentImage = shallowRef<ImageLightboxImage | null>(null);
let host: HTMLElement | null = null;

const ImageLightboxHost = defineComponent({
  name: "ImageLightboxHost",
  setup: () => () => h(ImageLightbox, { image: currentImage.value, onClose: closeImageLightbox }),
});

/**
 * Shows an image enlarged above every surface of the current window. The
 * overlay lives outside component trees, so editor widgets and `v-html`
 * renderers can open it without owning a template slot.
 */
export function openImageLightbox(image: ImageLightboxImage): void {
  if (!image.src || typeof document === "undefined") return;
  currentImage.value = { ...image };
  if (host?.isConnected) return;
  if (host) render(null, host);
  host = document.createElement("div");
  host.className = "image-lightbox-host";
  document.body.append(host);
  render(h(ImageLightboxHost), host);
}

export function closeImageLightbox(): void {
  currentImage.value = null;
}
