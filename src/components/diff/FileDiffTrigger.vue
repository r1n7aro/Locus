<script setup lang="ts">
import { ref, onUnmounted } from "vue";
import FileDiffPopover from "./FileDiffPopover.vue";
import { diffSingleFile, createRequestToken, isTokenStale } from "../../services/diff";
import { useDiffOverlay } from "../../composables/useDiffOverlay";
import type { GitFileChange, DiffSource, FileDiffPayload } from "../../types";

const props = defineProps<{
  fileChange: GitFileChange;
  source: DiffSource;
  commitHash?: string;
  sessionId?: string;
  assistantMessageId?: string;
}>();

const overlay = useDiffOverlay();

const triggerRef = ref<HTMLElement | null>(null);
const showPopover = ref(false);
const previewPayload = ref<FileDiffPayload | null>(null);

let hoverTimer: ReturnType<typeof setTimeout> | null = null;

function buildRequest(detail: "preview" | "full") {
  return {
    source: props.source,
    filePath: props.fileChange.path,
    oldPath: props.fileChange.oldPath,
    commitHash: props.commitHash,
    sessionId: props.sessionId,
    assistantMessageId: props.assistantMessageId,
    detail,
  };
}

function onMouseEnter() {
  hoverTimer = setTimeout(async () => {
    const token = createRequestToken();
    try {
      const payload = await diffSingleFile(buildRequest("preview"));
      if (isTokenStale(token)) return; // Stale — user already moved away
      previewPayload.value = payload;
      showPopover.value = true;
    } catch {
      // Silently ignore preview errors
    }
  }, 150);
}

function onMouseLeave() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  // Bump token to discard any in-flight preview response
  createRequestToken();
  showPopover.value = false;
  previewPayload.value = null;
}

async function onClick() {
  // Close popover
  showPopover.value = false;
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }

  try {
    const payload = await diffSingleFile(buildRequest("full"));
    overlay.open(payload);
  } catch (e) {
    console.error("[FileDiffTrigger] failed to fetch full diff:", e);
  }
}

function onPopoverClose() {
  showPopover.value = false;
  previewPayload.value = null;
}

onUnmounted(() => {
  if (hoverTimer) clearTimeout(hoverTimer);
});
</script>

<template>
  <div
    ref="triggerRef"
    class="diff-trigger"
    @mouseenter="onMouseEnter"
    @mouseleave="onMouseLeave"
    @click="onClick"
  >
    <slot />

    <FileDiffPopover
      v-if="showPopover && previewPayload && triggerRef"
      :payload="previewPayload"
      :anchor="triggerRef"
      @close="onPopoverClose"
    />
  </div>
</template>

<style scoped>
.diff-trigger {
  cursor: pointer;
}
.diff-trigger:hover {
  background: rgba(255, 255, 255, 0.04);
}
</style>
