<script setup lang="ts">
import { ref, onUnmounted, watch } from "vue";
import FileDiffPopover from "./FileDiffPopover.vue";
import { diffSingleFile, createRequestToken, isTokenStale } from "../../services/diff";
import { useDiffOverlay } from "../../composables/useDiffOverlay";
import { useDisplaySettings } from "../../composables/useDisplaySettings";
import type { GitFileChange, DiffSource, FileDiffPayload } from "../../types";

const props = defineProps<{
  fileChange: GitFileChange;
  source: DiffSource;
  commitHash?: string;
  sessionId?: string;
  assistantMessageId?: string;
}>();

const overlay = useDiffOverlay();
const { state: displaySettings } = useDisplaySettings();

const triggerRef = ref<HTMLElement | null>(null);
const showPopover = ref(false);
const previewPayload = ref<FileDiffPayload | null>(null);

let hoverTimer: ReturnType<typeof setTimeout> | null = null;
let closeTimer: ReturnType<typeof setTimeout> | null = null;
let hoverSeq = 0;
const HOVER_CLOSE_DELAY_MS = 140;

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
  if (!displaySettings.fileChangePopoverEnabled) return;
  cancelPopoverClose();
  if (showPopover.value && previewPayload.value) return;
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  const seq = ++hoverSeq;
  hoverTimer = setTimeout(async () => {
    const token = createRequestToken();
    try {
      const payload = await diffSingleFile(buildRequest("preview"));
      if (seq !== hoverSeq || isTokenStale(token)) return; // Stale — user already moved away
      previewPayload.value = payload;
      showPopover.value = true;
    } catch {
      // Silently ignore preview errors
    }
  }, 150);
}

function cancelPopoverClose() {
  if (closeTimer) {
    clearTimeout(closeTimer);
    closeTimer = null;
  }
}

function closePopover() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  if (closeTimer) {
    clearTimeout(closeTimer);
    closeTimer = null;
  }
  // Bump token to discard any in-flight preview response
  hoverSeq++;
  createRequestToken();
  showPopover.value = false;
  previewPayload.value = null;
}

watch(() => displaySettings.fileChangePopoverEnabled, (enabled) => {
  if (!enabled) closePopover();
});

function schedulePopoverClose() {
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  cancelPopoverClose();
  closeTimer = setTimeout(closePopover, HOVER_CLOSE_DELAY_MS);
}

function onMouseLeave() {
  schedulePopoverClose();
}

function onPopoverMouseEnter() {
  cancelPopoverClose();
}

function onPopoverMouseLeave() {
  schedulePopoverClose();
}

async function onClick() {
  // Close popover
  cancelPopoverClose();
  closePopover();

  try {
    const payload = await diffSingleFile(buildRequest("full"));
    overlay.open(payload);
  } catch (e) {
    console.error("[FileDiffTrigger] failed to fetch full diff:", e);
  }
}

function onPopoverClose() {
  closePopover();
}

onUnmounted(() => {
  if (hoverTimer) clearTimeout(hoverTimer);
  if (closeTimer) clearTimeout(closeTimer);
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
      @enter="onPopoverMouseEnter"
      @leave="onPopoverMouseLeave"
      @open="onClick"
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
