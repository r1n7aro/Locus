<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  deferredSessionMessageImageId,
  resolveToolResultImages,
} from "../composables/toolResultImages";
import { loadSessionMessageImages } from "../services/session";
import type { ImageAttachment } from "../types";

const props = defineProps<{
  images: ImageAttachment[];
}>();

const resolvedImages = ref<ImageAttachment[]>([]);

watch(
  () => props.images,
  async (images, _previous, onCleanup) => {
    let cancelled = false;
    onCleanup(() => {
      cancelled = true;
    });
    try {
      const resolved = await resolveToolResultImages(images, loadSessionMessageImages);
      if (!cancelled) resolvedImages.value = resolved;
    } catch (error) {
      console.warn("load_session_message_images failed:", error);
      if (!cancelled) {
        resolvedImages.value = images.filter((image) => !deferredSessionMessageImageId(image));
      }
    }
  },
  { immediate: true },
);

const validImages = computed(() =>
  resolvedImages.value.filter((image) => image.data && image.mimeType),
);

function imageDataUrl(image: ImageAttachment) {
  return `data:${image.mimeType};base64,${image.data}`;
}
</script>

<template>
  <div v-if="validImages.length > 0" class="tool-result-images">
    <div
      v-for="(image, index) in validImages"
      :key="`${image.mimeType}:${index}`"
      class="tool-result-image-frame"
    >
      <img
        class="tool-result-image"
        :src="imageDataUrl(image)"
        alt=""
      />
    </div>
  </div>
</template>

<style scoped>
.tool-result-images {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 6px;
  max-width: 100%;
}

.tool-result-image-frame {
  width: fit-content;
  max-width: 100%;
  padding: 4px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--hover-bg);
  overflow: hidden;
}

.tool-result-image {
  display: block;
  max-width: min(720px, 100%);
  max-height: 420px;
  object-fit: contain;
  border-radius: 4px;
}
</style>
