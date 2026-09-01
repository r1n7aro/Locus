<script setup lang="ts">
import { computed } from "vue";
import type { ImageAttachment } from "../../types";

const MAX_VISIBLE_IMAGES = 3;

const props = withDefaults(defineProps<{
  images?: ImageAttachment[];
}>(), {
  images: () => [],
});

const visibleImages = computed(() => props.images.slice(0, MAX_VISIBLE_IMAGES));
const hiddenImageCount = computed(() => Math.max(0, props.images.length - visibleImages.value.length));

function imageSource(image: ImageAttachment): string {
  return `data:${image.mimeType};base64,${image.data}`;
}
</script>

<template>
  <div v-if="images.length > 0" class="queued-follow-up-images">
    <img
      v-for="(image, index) in visibleImages"
      :key="`${image.mimeType}:${index}`"
      :src="imageSource(image)"
      class="queued-follow-up-image"
      alt=""
    />
    <span v-if="hiddenImageCount > 0" class="queued-follow-up-image-count">
      +{{ hiddenImageCount }}
    </span>
  </div>
</template>

<style scoped>
.queued-follow-up-images {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 3px;
}

.queued-follow-up-image {
  display: block;
  width: 24px;
  height: 24px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: color-mix(in srgb, var(--input-bg) 80%, var(--panel-bg) 20%);
  object-fit: cover;
}

.queued-follow-up-image-count {
  min-width: 18px;
  color: var(--text-secondary);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  text-align: center;
}
</style>
