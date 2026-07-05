
<script setup lang="ts">
import { shallowRef, watch } from "vue";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import {
  StreamingMarkdownSplitter,
  type StreamingMarkdownSplit,
} from "../../composables/streamingMarkdownBlocks";

/**
 * Streaming markdown surface with amortized O(n) total render cost.
 *
 * The growing text is split into frozen prefix blocks plus an active tail.
 * Frozen blocks keep stable ids and text, so their MarkdownRenderer
 * instances never re-render (parse, sanitize, injects, and DOM survive);
 * each frame re-renders only the tail. History rendering is untouched — the
 * one-shot full render after the round lands corrects any block-boundary
 * divergence accepted while streaming.
 */
const props = defineProps<{
  content: string;
  cursor?: boolean;
  enableFileRefs?: boolean;
  unityPreviewStateScope?: string | null;
}>();

const emit = defineEmits<{
  (e: "openImage", src: string): void;
}>();

/**
 * A tail longer than this renders as plain text instead of markdown — the
 * one case where per-frame cost could regress to O(document): a single
 * uncuttable block, e.g. a huge unclosed code fence. The full render after
 * the round completes restores proper formatting.
 */
const TAIL_MARKDOWN_LIMIT = 24_000;

const splitter = new StreamingMarkdownSplitter();
const split = shallowRef<StreamingMarkdownSplit>(splitter.update(props.content));

watch(
  () => props.content,
  (next) => {
    split.value = splitter.update(next);
  },
);

function blockPreviewScope(blockId: string): string | null | undefined {
  const scope = props.unityPreviewStateScope;
  return scope ? `${scope}:${blockId}` : scope;
}
</script>

<template>
  <div class="streaming-markdown">
    <MarkdownRenderer
      v-for="block in split.blocks"
      :key="block.id"
      class="streaming-markdown-block"
      :content="block.text"
      :enable-file-refs="enableFileRefs"
      :unity-preview-state-scope="blockPreviewScope(block.id)"
      @open-image="emit('openImage', $event)"
    />
    <pre
      v-if="split.tail && split.tail.length > TAIL_MARKDOWN_LIMIT"
      class="streaming-markdown-block streaming-markdown-tail-plain ui-select-text"
    >{{ split.tail }}</pre>
    <MarkdownRenderer
      v-else-if="split.tail"
      class="streaming-markdown-block"
      :content="split.tail"
      :cursor="cursor"
      :enable-file-refs="enableFileRefs"
      :unity-preview-state-scope="blockPreviewScope('tail')"
      @open-image="emit('openImage', $event)"
    />
  </div>
</template>

<style scoped>
/* Frozen blocks each end with `> :last-child { margin-bottom: 0 }` from
 * .markdown-body, so restore the paragraph rhythm between blocks. */
.streaming-markdown-block:not(:last-child) {
  margin-bottom: 12px;
}

/* Headings opening a block lose their 24px top margin to
 * `> :first-child { margin-top: 0 }`; the extra 12px on top of the block
 * gap restores the full-document spacing. */
.streaming-markdown-block + .streaming-markdown-block :deep(> h1:first-child),
.streaming-markdown-block + .streaming-markdown-block :deep(> h2:first-child),
.streaming-markdown-block + .streaming-markdown-block :deep(> h3:first-child),
.streaming-markdown-block + .streaming-markdown-block :deep(> h4:first-child),
.streaming-markdown-block + .streaming-markdown-block :deep(> h5:first-child),
.streaming-markdown-block + .streaming-markdown-block :deep(> h6:first-child) {
  margin-top: 12px;
}

.streaming-markdown-tail-plain {
  margin: 0;
  font-family: var(--font-prose);
  font-size: 14px;
  line-height: 1.68;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-color);
}
</style>
