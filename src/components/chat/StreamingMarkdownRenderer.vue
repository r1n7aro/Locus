
<script setup lang="ts">
import { computed, onBeforeUnmount, shallowRef, watch } from "vue";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import {
  StreamingMarkdownSplitter,
  type StreamingMarkdownSplit,
} from "../../composables/streamingMarkdownBlocks";
import { STREAMING_RENDER_THROTTLE_MS } from "../../composables/streamingRenderThrottle";
import {
  collectMarkdownPathCandidates,
  createMarkdownPathCandidateScanner,
  streamingTrailingPathRunStart,
} from "../../composables/markdownInject";
import { prefetchMarkdownPathCandidates } from "../../composables/markdownPathStatusPrefetch";
import type { StreamingTextSource } from "../../composables/streamingTextChunks";
import type { WorkspaceRef } from "../../services/project";
import type { Citation } from "../../types";

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
  citations?: Citation[];
  stream?: StreamingTextSource | null;
  streamInitial?: string;
  /** Full response has landed. Deferred atomic tails render from their final
   * source even when the closing fence is the document's last unterminated
   * line. */
  finalized?: boolean;
  cursor?: boolean;
  enableFileRefs?: boolean;
  unityPreviewStateScope?: string | null;
  workspaceRef?: WorkspaceRef | null;
}>();

const emit = defineEmits<{
  (e: "openImage", src: string): void;
  (e: "layoutChange"): void;
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
function visibleTail(next: StreamingMarkdownSplit): string {
  if (props.finalized || next.deferredTailStart === undefined) return next.tail;
  return next.tail.slice(0, next.deferredTailStart);
}

const renderTail = computed(() => {
  return visibleTail(split.value);
});

const citationContent = computed(() => {
  if (!props.citations?.length) return "";
  if (props.content !== undefined) return props.content;
  return `${props.streamInitial ?? ""}${props.stream?.full() ?? ""}`;
});

// Vue post-flush watchers run after this component's DOM patch and before the
// browser paints. Let the transcript correct its viewport against the actual
// Markdown height instead of the parent's independently throttled text value.
watch(split, () => emit("layoutChange"), { flush: "post" });

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
  projectPlainTail(visibleTail(next));
}

watch(
  () => props.finalized,
  () => projectPlainTail(renderTail.value),
);

// -- path references --
//
// A path renders as a file reference only once its status says it exists. So
// that streamed paths do not show as text first and as references a frame
// later:
// - stream mode prefetches statuses as each delta arrives, so they are
//   usually cached before the next render;
// - a growth render whose new text still has unknown statuses waits for
//   them, at most PATH_STATUS_RENDER_WAIT_MS;
// - the trailing run of path characters, which the next delta may still
//   extend into a path, is held back until a delimiter arrives, the text stops
//   growing for HELD_RUN_IDLE_RELEASE_MS, or the text is finalized. Held text
//   is never given to the splitter, so shown text only ever grows.

/** Longest a growth render waits for the path statuses of its new text. */
const PATH_STATUS_RENDER_WAIT_MS = STREAMING_RENDER_THROTTLE_MS;
/** A held trailing run shows once the text has not grown for this long. */
const HELD_RUN_IDLE_RELEASE_MS = 250;

const pathScanOptions = {
  get fileRefs() {
    return !!props.enableFileRefs;
  },
};
const holdsTrailingRuns = computed(() => !!props.enableFileRefs && !props.finalized);

let pathWaitRun = 0;
let pathWaitStartedAt = 0;
let pathWaitTimer: ReturnType<typeof setTimeout> | null = null;

function cancelPathWait() {
  pathWaitRun += 1;
  pathWaitStartedAt = 0;
  if (pathWaitTimer !== null) clearTimeout(pathWaitTimer);
  pathWaitTimer = null;
}

/**
 * Runs `action` once `pending` settles, or when the wait that started with the
 * first still-unresolved call runs out of budget. A newer call supersedes the
 * pending action, so the newest split is the one that renders.
 */
function afterPathStatuses(pending: Promise<void> | null, action: () => void) {
  if (pathWaitTimer !== null) clearTimeout(pathWaitTimer);
  pathWaitTimer = null;
  const run = ++pathWaitRun;
  const now = performance.now();
  if (pending && !pathWaitStartedAt) pathWaitStartedAt = now;
  const remaining = pending ? PATH_STATUS_RENDER_WAIT_MS - (now - pathWaitStartedAt) : 0;
  if (!pending || remaining <= 0) {
    pathWaitStartedAt = 0;
    action();
    return;
  }
  const finish = () => {
    if (run !== pathWaitRun) return;
    cancelPathWait();
    action();
  };
  pathWaitTimer = setTimeout(finish, remaining);
  void pending.then(finish, finish);
}

/** Prefetches the statuses of text a split would show for the first time. */
function pathStatusPrefetchFor(next: StreamingMarkdownSplit): Promise<void> | null {
  const shownBlockIds = new Set(split.value.blocks.map((block) => block.id));
  const candidates: string[] = [];
  for (const block of next.blocks) {
    if (shownBlockIds.has(block.id)) continue;
    candidates.push(...collectMarkdownPathCandidates(block.text, pathScanOptions));
  }
  const tail = visibleTail(next);
  // An oversized tail renders as plain text, without references.
  if (tail.length <= TAIL_MARKDOWN_LIMIT) {
    candidates.push(...collectMarkdownPathCandidates(tail, pathScanOptions));
  }
  return prefetchMarkdownPathCandidates(candidates, props.workspaceRef);
}

function commitGrowth(next: StreamingMarkdownSplit) {
  afterPathStatuses(pathStatusPrefetchFor(next), () => applySplit(next));
}

function commitNow(next: StreamingMarkdownSplit) {
  cancelPathWait();
  applySplit(next);
}

let idleReleaseTimer: ReturnType<typeof setTimeout> | null = null;

function clearIdleRelease() {
  if (idleReleaseTimer !== null) clearTimeout(idleReleaseTimer);
  idleReleaseTimer = null;
}

// -- content mode --

/** Prefix of `content` given to the splitter; the rest is a held trailing run. */
let contentShown = "";

function settleContent(next: string, release: boolean): { split: StreamingMarkdownSplit; grew: boolean } {
  const grew = contentShown.length > 0 && next.length > contentShown.length && next.startsWith(contentShown);
  let end = next.length;
  if (grew && !release && holdsTrailingRuns.value) {
    end = Math.max(contentShown.length, streamingTrailingPathRunStart(next));
  }
  contentShown = end === next.length ? next : next.slice(0, end);
  return { split: splitter.update(contentShown), grew };
}

watch(
  () => props.content,
  (next) => {
    if (props.stream) return;
    const settled = settleContent(next ?? "", false);
    if (settled.grew) commitGrowth(settled.split);
    else commitNow(settled.split);
    scheduleIdleRelease();
  },
);

// -- stream mode --

let streamCursor = 0;
let streamGeneration = -1;
let streamFlushTimer: ReturnType<typeof setTimeout> | null = null;
/** Received stream text not yet given to the splitter (a held trailing run). */
let heldStreamText = "";

const pathScanner = createMarkdownPathCandidateScanner(pathScanOptions);
let prescanCursor = 0;
let prescanGeneration = -1;

function clearStreamFlushTimer() {
  if (streamFlushTimer === null) return;
  clearTimeout(streamFlushTimer);
  streamFlushTimer = null;
}

function prefetchScannedPaths(candidates: string[]) {
  if (candidates.length > 0) void prefetchMarkdownPathCandidates(candidates, props.workspaceRef);
}

/** Starts resolving the paths of newly arrived text, ahead of the next render. */
function prescanStream(source: StreamingTextSource) {
  if (source.generation !== prescanGeneration || source.length < prescanCursor) {
    prescanGeneration = source.generation;
    prescanCursor = 0;
    pathScanner.reset();
    const initial = props.streamInitial ?? "";
    if (initial) prefetchScannedPaths(pathScanner.feed(initial));
  }
  if (source.length === prescanCursor) return;
  const delta = source.readFrom(prescanCursor);
  prescanCursor = source.length;
  prefetchScannedPaths(pathScanner.feed(delta));
}

function appendSettled(delta: string, release: boolean): StreamingMarkdownSplit {
  const text = heldStreamText + delta;
  const end = release || !holdsTrailingRuns.value ? text.length : streamingTrailingPathRunStart(text);
  heldStreamText = text.slice(end);
  return splitter.append(end === text.length ? text : text.slice(0, end));
}

function consumeStream(source: StreamingTextSource, release = false) {
  prescanStream(source);
  let reset = false;
  if (source.generation !== streamGeneration || source.length < streamCursor) {
    streamGeneration = source.generation;
    streamCursor = 0;
    heldStreamText = "";
    splitter.reset();
    resetPlainTail();
    const initial = props.streamInitial ?? "";
    if (initial) {
      splitter.append(initial);
    } else {
      splitter.update("");
    }
    reset = true;
  }
  const delta = source.readFrom(streamCursor);
  streamCursor = source.length;
  const next = appendSettled(delta, release);
  // A reset re-renders from scratch; only growth waits for path statuses.
  if (reset) commitNow(next);
  else commitGrowth(next);
  scheduleIdleRelease();
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
        heldStreamText = "";
        splitter.reset();
        resetPlainTail();
        contentShown = props.content ?? "";
        commitNow(splitter.update(contentShown));
      }
      return;
    }
    contentShown = "";
    streamGeneration = -1;
    prescanGeneration = -1;
    consumeStream(source);
  },
  { immediate: true },
);

watch(
  () => props.stream?.version.value,
  () => {
    if (!props.stream) return;
    prescanStream(props.stream);
    scheduleStreamFlush(props.stream);
  },
);

// -- held trailing runs --

function hasHeldText(): boolean {
  if (props.stream) return heldStreamText.length > 0;
  return contentShown.length < (props.content ?? "").length;
}

function releaseHeldText() {
  clearIdleRelease();
  if (!hasHeldText()) return;
  if (props.stream) {
    consumeStream(props.stream, true);
    return;
  }
  commitGrowth(settleContent(props.content ?? "", true).split);
}

function scheduleIdleRelease() {
  clearIdleRelease();
  if (!hasHeldText()) return;
  idleReleaseTimer = setTimeout(releaseHeldText, HELD_RUN_IDLE_RELEASE_MS);
}

watch(holdsTrailingRuns, (holds) => {
  if (!holds) releaseHeldText();
});

// Seed content mode once on mount (stream mode seeds through its own watch).
if (!props.stream) {
  applySplit(settleContent(props.content ?? "", true).split);
}

onBeforeUnmount(() => {
  clearStreamFlushTimer();
  clearIdleRelease();
  cancelPathWait();
});

function blockPreviewScope(blockId: string): string | null | undefined {
  const scope = props.unityPreviewStateScope;
  return scope ? `${scope}:${blockId}` : scope;
}
</script>

<template>
  <div class="streaming-markdown">
    <MarkdownRenderer
      v-if="citations?.length"
      class="streaming-markdown-block"
      :content="citationContent"
      :citations="citations"
      :enable-file-refs="enableFileRefs"
      :workspace-ref="workspaceRef"
      :unity-preview-state-scope="blockPreviewScope('citations')"
      @open-image="emit('openImage', $event)"
    />
    <template v-else>
      <!-- `markdown-segment` keeps inner edge margins so block boundaries lay
        out like one document wherever the splitter cuts. -->
      <MarkdownRenderer
        v-for="block in split.blocks"
        :key="block.id"
        class="streaming-markdown-block markdown-segment"
        :content="block.text"
        :enable-file-refs="enableFileRefs"
        :workspace-ref="workspaceRef"
        :unity-preview-state-scope="blockPreviewScope(block.id)"
        @open-image="emit('openImage', $event)"
      />
      <pre
        v-if="renderTail && renderTail.length > TAIL_MARKDOWN_LIMIT"
        class="streaming-markdown-block streaming-markdown-tail-plain ui-select-text"
      ><span
        v-for="(part, index) in plainTailParts"
        :key="index"
      >{{ part }}</span><span>{{ plainTailActive }}</span></pre>
      <MarkdownRenderer
        v-else-if="renderTail"
        class="streaming-markdown-block markdown-segment"
        :content="renderTail"
        :cursor="cursor"
        :enable-file-refs="enableFileRefs"
        :workspace-ref="workspaceRef"
        :unity-preview-state-scope="blockPreviewScope('tail')"
        @open-image="emit('openImage', $event)"
      />
    </template>
  </div>
</template>

<style scoped>
/* Block spacing needs no rules here: the wrappers neither pad nor form a
 * formatting context, so the `markdown-segment` edge margins collapse across
 * them (paragraph -> heading stays 24px, heading -> paragraph 10px). Fixed
 * gaps diverged from that and moved the text whenever a cut moved, e.g.
 * when the final render re-split the finished answer. */
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
