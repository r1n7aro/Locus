<script setup lang="ts">
import type { IconNode } from "lucide";

defineOptions({
  inheritAttrs: false,
});

withDefaults(defineProps<{
  icon: IconNode;
  size?: number | string;
  strokeWidth?: number | string;
}>(), {
  size: 14,
  strokeWidth: 2,
});
</script>

<template>
  <svg
    v-bind="$attrs"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="strokeWidth"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <component
      :is="tag"
      v-for="([tag, attrs], idx) in icon"
      :key="idx"
      v-bind="attrs"
    />
  </svg>
</template>
