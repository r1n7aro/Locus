
<script setup lang="ts">
import { onBeforeUnmount, shallowRef, watch } from "vue";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import {
  StreamingMarkdownSplitter,
  type StreamingMarkdownSplit,
} from "../../composables/streamingMarkdownBlocks";
import { STREAMING_RENDER_THROTTLE_MS } from "../../composables/streamingRenderThrottle";
import type { StreamingTextSource } from "../../composables/streamingTextChunks";

/**
 * Streaming markdown surface with amortized O(n) total render cost.
 *
 * The growing text is split into frozen prefix blocks plus an active tail.
 * Frozen blocks keep stable ids and text, so their MarkdownRenderer
 * instances never re-render (parse, sanitize, injects, and DOM survive);
 * each frame re-renders only the tail. History rendering is untouched — the
 * one-shot full render after the round lands corrects any block-boundary
 * divergence accepted while streaming.
 *
 * Two input modes, mutually exclusive:
 * - `content`: the caller re-materializes the full text per update. Kept for
 *   low-frequency surfaces and history correction.
 * - `stream`: an append-only chunk buffer with a stable identity. Growth is
 *   consumed incrementally at the shared streaming cadence, so neither this
 *   component nor its parent re-renders per delta, and no full-text string is
 *   ever rebuilt while streaming. `streamInitial` seeds text that accumulated
 *   before the buffer existed (runtime snapshot restores).
 */
const props = defineProps<{
  content?: string;
  stream?: StreamingTextSource | null;
  streamInitial?: string;
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

/** Plain-tail parts freeze at this size so Blink only ever lays out the
 * growing remainder, not the whole oversized block. */
const PLAIN_TAIL_PART_TARGET = 4096;

const splitter = new StreamingMarkdownSplitter();
const split = shallowRef<StreamingMarkdownSplit>({ blocks: [], tail: "" });

/**
 * Append-only projection of an oversized plain tail: frozen spans plus a
 * growing remainder, so DOM text nodes for already-shown output never change.
 */
const plainTailParts = shallowRef<readonly string[]>([]);
const plainTailActive = shallowRef("");
let plainTailConsumed = 0;
let plainTailBlockCount = 0;

function resetPlainTail() {
  plainTailParts.value = [];
  plainTailActive.value = "";
  plainTailConsumed = 0;
}

function projectPlainTail(tail: string) {
  if (tail.length <= TAIL_MARKDOWN_LIMIT) {
    if (plainTailConsumed > 0 || plainTailActive.value) resetPlainTail();
    return;
  }
  if (tail.length < plainTailConsumed + plainTailActive.value.length) {
    resetPlainTail();
  }
  let active = plainTailActive.value;
  const appended = tail.slice(plainTailConsumed + active.length);
  if (!appended) return;
  active += appended;
  if (active.length >= PLAIN_TAIL_PART_TARGET) {
    plainTailParts.value = [...plainTailParts.value, active];
    plainTailConsumed += active.length;
    active = "";
  }
  plainTailActive.value = active;
}

function applySplit(next: StreamingMarkdownSplit) {
  // A committed block re-heads the tail. The projection assumes it holds a
  // prefix of the current tail, which a length check alone cannot verify when
  // one flush both cuts a block and appends at least as much new text — so
  // rebuild whenever the block count moves.
  if (next.blocks.length !== plainTailBlockCount) {
    plainTailBlockCount = next.blocks.length;
    resetPlainTail();
  }
  split.value = next;
  projectPlainTail(next.tail);
}

// -- content mode --

watch(
  () => props.content,
  (next) => {
    if (props.stream) return;
    applySplit(splitter.update(next ?? ""));
  },
);

// -- stream mode --

let streamCursor = 0;
let streamGeneration = -1;
let streamFlushTimer: ReturnType<typeof setTimeout> | null = null;

function clearStreamFlushTimer() {
  if (streamFlushTimer === null) return;
  clearTimeout(streamFlushTimer);
  streamFlushTimer = null;
}

function consumeStream(source: StreamingTextSource) {
  if (source.generation !== streamGeneration || source.length < streamCursor) {
    streamGeneration = source.generation;
    streamCursor = 0;
    splitter.reset();
    resetPlainTail();
    const initial = props.streamInitial ?? "";
    if (initial) {
      applySplit(splitter.append(initial));
    } else {
      applySplit(splitter.update(""));
    }
  }
  const delta = source.readFrom(streamCursor);
  streamCursor = source.length;
  if (delta || split.value.tail || split.value.blocks.length) {
    applySplit(splitter.append(delta));
  }
}

function scheduleStreamFlush(source: StreamingTextSource) {
  if (streamFlushTimer !== null) return;
  streamFlushTimer = setTimeout(() => {
    streamFlushTimer = null;
    if (props.stream === source) consumeStream(source);
  }, STREAMING_RENDER_THROTTLE_MS);
}

watch(
  () => props.stream,
  (source, previous) => {
    clearStreamFlushTimer();
    if (!source) {
      if (previous) {
        splitter.reset();
        resetPlainTail();
        applySplit(splitter.update(props.content ?? ""));
      }
      return;
    }
    streamGeneration = -1;
    consumeStream(source);
  },
  { immediate: true },
);

watch(
  () => props.stream?.version.value,
  () => {
    if (!props.stream) return;
    scheduleStreamFlush(props.stream);
  },
);

// Seed content mode once on mount (stream mode seeds through its own watch).
if (!props.stream) {
  applySplit(splitter.update(props.content ?? ""));
}

onBeforeUnmount(clearStreamFlushTimer);

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
    ><span
      v-for="(part, index) in plainTailParts"
      :key="index"
    >{{ part }}</span><span>{{ plainTailActive }}</span></pre>
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
