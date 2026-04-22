<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from "vue";
import hljs from "../../hljs";
import type { WorkspaceFilePreview } from "../../services/unity";

const props = defineProps<{
  preview: WorkspaceFilePreview;
  anchor: HTMLElement;
  unityConnected: boolean;
}>();

const emit = defineEmits<{
  close: [];
  cancelClose: [];
  scheduleClose: [];
}>();

const popoverRef = ref<HTMLElement | null>(null);
const style = ref({ top: "0px", left: "0px" });

function updatePosition() {
  if (!props.anchor || !popoverRef.value) return;
  const rect = props.anchor.getBoundingClientRect();
  const popRect = popoverRef.value.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  let top = rect.bottom + 4;
  let left = rect.left;

  if (top + popRect.height > vh && rect.top - popRect.height - 4 > 0) {
    top = rect.top - popRect.height - 4;
  }
  if (left + popRect.width > vw) {
    left = vw - popRect.width - 8;
  }
  if (left < 8) left = 8;

  style.value = { top: `${top}px`, left: `${left}px` };
}

let scrollParents: Element[] = [];

function findScrollParents(el: Element | null): Element[] {
  const parents: Element[] = [];
  let current = el?.parentElement;
  while (current) {
    const overflow = getComputedStyle(current).overflowY;
    if (overflow === "auto" || overflow === "scroll") {
      parents.push(current);
    }
    current = current.parentElement;
  }
  return parents;
}

function onScroll() {
  emit("close");
}

onMounted(() => {
  nextTick(updatePosition);
  scrollParents = findScrollParents(props.anchor);
  scrollParents.forEach((p) => p.addEventListener("scroll", onScroll, { passive: true }));
});

onUnmounted(() => {
  scrollParents.forEach((p) => p.removeEventListener("scroll", onScroll));
});

watch(() => props.anchor, () => nextTick(updatePosition));

const highlightedSnippet = computed(() => {
  const p = props.preview;
  if (p.kind !== "text" || !p.snippet) return "";
  const lang = p.language;
  let highlighted: string;
  if (lang && hljs.getLanguage(lang)) {
    try {
      highlighted = hljs.highlight(p.snippet, { language: lang }).value;
    } catch {
      highlighted = escapeHtml(p.snippet);
    }
  } else {
    highlighted = escapeHtml(p.snippet);
  }
  const lines = highlighted.split("\n");
  const startLine = p.snippetStartLine || 1;
  return lines
    .map(
      (line, i) =>
        `<span class="preview-line"><span class="preview-ln">${startLine + i}</span><span class="preview-lc">${line || " "}</span></span>`,
    )
    .join("\n");
});

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function formatSize(bytes?: number): string {
  if (bytes == null) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const actionHint = computed(() => {
  const p = props.preview;
  if (p.preferredAction === "editor") return "Click to open in editor";
  if (p.preferredAction === "unity" && props.unityConnected) return "Click to select in Unity";
  return "Click to open with default app";
});
</script>

<template>
  <Teleport to="body">
    <div
      ref="popoverRef"
      class="file-preview-popover"
      :style="style"
      @mouseenter="emit('cancelClose')"
      @mouseleave="emit('scheduleClose')"
    >
      <div class="popover-header">
        <span class="file-path">{{ preview.displayPath }}</span>
        <span v-if="preview.language" class="lang-tag">{{ preview.language }}</span>
      </div>

      <div v-if="preview.kind === 'text' && preview.snippet" class="popover-body">
        <pre><code v-html="highlightedSnippet" /></pre>
        <div v-if="preview.truncated" class="truncation-hint">...</div>
      </div>
      <div v-else-if="preview.kind === 'binary'" class="popover-body binary-hint">
        Binary file{{ preview.fileSize ? ` \u00B7 ${formatSize(preview.fileSize)}` : "" }}
      </div>
      <div v-else class="popover-body binary-hint">
        File not found
      </div>

      <div class="popover-hint">{{ actionHint }}</div>
    </div>
  </Teleport>
</template>

<style scoped>
.file-preview-popover {
  position: fixed;
  z-index: 150;
  width: 420px;
  max-height: 240px;
  background: var(--sidebar-bg);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.popover-header {
  padding: 6px 10px;
  font-size: 12px;
  color: var(--text-primary);
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 0;
}

.file-path {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}

.lang-tag {
  font-size: 10px;
  padding: 1px 5px;
  border-radius: 3px;
  background: var(--hover-bg);
  color: var(--text-secondary);
  flex-shrink: 0;
}

.popover-body {
  flex: 1;
  overflow: auto;
  min-height: 0;
}

.popover-body pre {
  margin: 0;
  padding: 0;
  background: transparent;
}

.popover-body code {
  display: block;
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.5;
  padding: 6px 0;
  white-space: pre;
  overflow-x: auto;
}

.preview-line {
  display: block;
}

.preview-ln {
  display: inline-block;
  width: 3em;
  padding-right: 8px;
  text-align: right;
  color: var(--line-number-color, #6e7681);
  user-select: none;
  opacity: 0.6;
  font-size: 11px;
}

.preview-lc {
  padding-left: 4px;
}

.truncation-hint {
  text-align: center;
  font-size: 11px;
  color: var(--text-secondary);
  padding: 2px 0 4px;
  opacity: 0.5;
}

.binary-hint {
  padding: 16px;
  text-align: center;
  font-size: 12px;
  color: var(--text-secondary);
}

.popover-hint {
  padding: 4px 10px;
  font-size: 10px;
  color: var(--text-secondary);
  text-align: center;
  border-top: 1px solid var(--border-color);
  opacity: 0.6;
}
</style>
