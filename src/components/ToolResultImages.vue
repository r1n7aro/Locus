<script setup lang="ts">
import { computed } from "vue";
import type { ImageAttachment } from "../types";

const props = defineProps<{
  images: ImageAttachment[];
}>();

const validImages = computed(() =>
  props.images.filter((image) => image.data && image.mimeType),
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
