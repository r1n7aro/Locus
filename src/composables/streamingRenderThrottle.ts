import { onScopeDispose, ref, watch, type Ref } from "vue";
import type { AssistantRenderPart } from "../types";

/**
 * Shared cadence for every surface that re-renders markdown from a growing
 * stream (assistant text parts, tool output). Matches the trailing flush that
 * ChatView applies to `displayedStreamingText` so all streaming surfaces
 * repaint in the same rhythm instead of once per delta event.
 */
export const STREAMING_RENDER_THROTTLE_MS = 80;

export interface ThrottledStreamingText {
  text: Readonly<Ref<string>>;
  /** Apply the latest pending value immediately (e.g. when a stream finishes). */
  flush: () => void;
}

/**
 * Trailing throttle for append-only streaming text. Growth is coalesced to
 * one update per `delayMs`; resets and replacements with shorter text flush
 * immediately so stale content never outlives its source.
 */
export function useThrottledStreamingText(
  source: () => string,
  delayMs = STREAMING_RENDER_THROTTLE_MS,
): ThrottledStreamingText {
  const displayed = ref(source());
  let pending = displayed.value;
  let timer: ReturnType<typeof setTimeout> | null = null;

  function clearTimer() {
    if (timer === null) return;
    clearTimeout(timer);
    timer = null;
  }

  // flush() re-reads the source so external callers (e.g. a status watcher
  // firing before this composable's own watcher in the same flush cycle)
  // always apply the latest value, not a stale pending snapshot.
  function flush() {
    clearTimer();
    pending = source();
    if (displayed.value !== pending) {
      displayed.value = pending;
    }
  }

  watch(source, (next) => {
    pending = next;
    if (!next || next.length < displayed.value.length) {
      flush();
      return;
    }
    if (timer !== null) return;
    timer = setTimeout(flush, delayMs);
  });

  onScopeDispose(clearTimer);

  return { text: displayed, flush };
}

function textLikeContentLength(part: AssistantRenderPart): number | null {
  return part.kind === "text" || part.kind === "thinking" ? part.content.length : null;
}

/**
 * Growth that only appends to text/thinking content keeps the same part ids,
 * kinds, and positions. Anything else (parts added/removed/reordered, content
 * replaced with something shorter) is a structural change that must render
 * immediately — e.g. clearing live parts when a round completes, otherwise the
 * transient block would briefly duplicate the message just pushed to history.
 */
function isAppendOnlyGrowth(
  next: readonly AssistantRenderPart[],
  displayed: readonly AssistantRenderPart[],
): boolean {
  if (next.length !== displayed.length) return false;
  for (let index = 0; index < next.length; index += 1) {
    const nextPart = next[index]!;
    const displayedPart = displayed[index]!;
    if (nextPart.id !== displayedPart.id || nextPart.kind !== displayedPart.kind) {
      return false;
    }
    const nextLength = textLikeContentLength(nextPart);
    const displayedLength = textLikeContentLength(displayedPart);
    if (nextLength !== null && displayedLength !== null && nextLength < displayedLength) {
      return false;
    }
  }
  return true;
}

/**
 * Trailing throttle for live assistant render parts. Per-delta content growth
 * coalesces to one update per `delayMs`; structural changes flush immediately
 * so parts appear/disappear in sync with the raw stream state.
 */
export function useThrottledLiveRenderParts(
  source: () => AssistantRenderPart[],
  delayMs = STREAMING_RENDER_THROTTLE_MS,
): Readonly<Ref<AssistantRenderPart[]>> {
  const displayed = ref(source()) as Ref<AssistantRenderPart[]>;
  let pending = displayed.value;
  let timer: ReturnType<typeof setTimeout> | null = null;

  function clearTimer() {
    if (timer === null) return;
    clearTimeout(timer);
    timer = null;
  }

  function flush() {
    clearTimer();
    if (displayed.value !== pending) {
      displayed.value = pending;
    }
  }

  watch(source, (next) => {
    pending = next;
    if (!isAppendOnlyGrowth(next, displayed.value)) {
      flush();
      return;
    }
    if (timer !== null) return;
    timer = setTimeout(flush, delayMs);
  });

  onScopeDispose(clearTimer);

  return displayed;
}
