import type { ChatMessage, ToolCallDisplay } from "../types";
import { isNearBottom, type ScrollMetrics, type SessionScrollState } from "./chatScrollState";

type ToolCallRuntimeStatus = Pick<ToolCallDisplay, "status" | "nestedToolCalls">;

export function shouldShowAssistantContinuation(
  lastGroupRole: "user" | "assistant" | null,
  hasTransientAssistantMessage: boolean,
): boolean {
  return hasTransientAssistantMessage && lastGroupRole === "assistant";
}

export interface PendingContinuationToolItem {
  id: string;
  content: string;
  toolCallCount: number;
}

export interface PendingContinuationRenderSegment {
  type: "toolCalls" | "content" | "other";
  itemIds?: readonly string[];
}

type TrailingToolMessage = Pick<ChatMessage, "id" | "role" | "toolCalls">;

export function findTrailingAssistantToolMessageId(
  messages: TrailingToolMessage[],
): string | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!message || message.role === "tool") continue;
    if (message.role !== "assistant") return null;
    return message.toolCalls && message.toolCalls.length > 0 ? message.id : null;
  }

  return null;
}

export function collectPendingContinuationToolItemIds(params: {
  isStreaming: boolean;
  lastGroupRole: "user" | "assistant" | null;
  hasTransientAssistantMessage: boolean;
  items: PendingContinuationToolItem[];
}): Set<string> {
  const {
    isStreaming,
    lastGroupRole,
    hasTransientAssistantMessage,
    items,
  } = params;

  if (!isStreaming || !shouldShowAssistantContinuation(lastGroupRole, hasTransientAssistantMessage)) {
    return new Set();
  }

  const pendingIds = new Set<string>();
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (!item) continue;
    if (item.content.trim().length > 0) break;
    if (item.toolCallCount > 0) {
      pendingIds.add(item.id);
    }
  }

  return pendingIds;
}

export function collectPendingContinuationToolSegmentItemIds(params: {
  isStreaming: boolean;
  lastGroupRole: "user" | "assistant" | null;
  hasTransientAssistantMessage: boolean;
  segments: PendingContinuationRenderSegment[];
}): Set<string> {
  const {
    isStreaming,
    lastGroupRole,
    hasTransientAssistantMessage,
    segments,
  } = params;

  if (!isStreaming || !shouldShowAssistantContinuation(lastGroupRole, hasTransientAssistantMessage)) {
    return new Set();
  }

  const pendingIds = new Set<string>();
  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index];
    if (!segment) continue;
    if (segment.type === "content") break;
    if (segment.type !== "toolCalls") continue;
    for (const itemId of segment.itemIds ?? []) {
      pendingIds.add(itemId);
    }
  }

  return pendingIds;
}

export function shouldAutoScrollToBottom(params: {
  force?: boolean;
  metrics: ScrollMetrics;
  remembered: SessionScrollState | null | undefined;
}): boolean {
  const { force = false, metrics, remembered } = params;
  if (!force && remembered && remembered.mode !== "bottom" && !isNearBottom(metrics)) {
    return false;
  }
  return true;
}

export function shouldShowWaitingPlaceholder(params: {
  isStreaming: boolean;
  hasStreamingContent: boolean;
  isThinking: boolean;
  hasThinkingContent: boolean;
}): boolean {
  const {
    isStreaming,
    hasStreamingContent,
    isThinking,
    hasThinkingContent,
  } = params;

  return (
    isStreaming
    && !hasStreamingContent
    && !isThinking
    && !hasThinkingContent
  );
}

export function hasRunningToolCall(toolCalls: ToolCallRuntimeStatus[]): boolean {
  return toolCalls.some((toolCall) =>
    toolCall.status === "running"
    || hasRunningToolCall(toolCall.nestedToolCalls ?? []),
  );
}

type FrameRequest = (cb: FrameRequestCallback) => number;
type FrameCancel = (id: number) => void;
type TimeoutRequest = (cb: () => void, delay: number) => number;
type TimeoutCancel = (id: number) => void;

export const CHAT_USER_SCROLL_INTENT_TTL_MS = 1400;

export function createUserScrollIntentTracker(
  now: () => number = () => (typeof performance !== "undefined" ? performance.now() : Date.now()),
  ttlMs = CHAT_USER_SCROLL_INTENT_TTL_MS,
) {
  let lastIntentAt = Number.NEGATIVE_INFINITY;

  return {
    mark() {
      lastIntentAt = now();
    },
    isRecent(currentTime = now()) {
      return currentTime - lastIntentAt <= ttlMs;
    },
    clear() {
      lastIntentAt = Number.NEGATIVE_INFINITY;
    },
    lastIntentAt() {
      return lastIntentAt;
    },
  };
}

function defaultFrameRequest(cb: FrameRequestCallback): number {
  if (typeof requestAnimationFrame === "function") {
    return requestAnimationFrame(cb);
  }
  if (typeof window !== "undefined") {
    return window.setTimeout(() => cb(Date.now()), 16);
  }
  return globalThis.setTimeout(() => cb(Date.now()), 16) as unknown as number;
}

function defaultFrameCancel(id: number) {
  if (typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(id);
    return;
  }
  clearTimeout(id);
}

function defaultTimeoutRequest(cb: () => void, delay: number): number {
  if (typeof window !== "undefined") {
    return window.setTimeout(cb, delay);
  }
  return globalThis.setTimeout(cb, delay) as unknown as number;
}

function defaultTimeoutCancel(id: number) {
  if (typeof window !== "undefined") {
    window.clearTimeout(id);
    return;
  }
  globalThis.clearTimeout(id as unknown as ReturnType<typeof setTimeout>);
}

export function createCoalescedScrollScheduler(
  run: (force: boolean) => void,
  requestFrame: FrameRequest = defaultFrameRequest,
  cancelFrame: FrameCancel = defaultFrameCancel,
) {
  let frameId = 0;
  let pendingForce = false;

  function flush() {
    if (!frameId) return;
    frameId = 0;
    const force = pendingForce;
    pendingForce = false;
    run(force);
  }

  return {
    schedule(force = false) {
      pendingForce = pendingForce || force;
      if (frameId) return;
      frameId = requestFrame(() => flush());
    },
    cancel() {
      if (!frameId) return;
      cancelFrame(frameId);
      frameId = 0;
      pendingForce = false;
    },
  };
}

export function createSettledScrollScheduler(
  run: () => void,
  settleDelayMs: number,
  requestFrame: FrameRequest = defaultFrameRequest,
  cancelFrame: FrameCancel = defaultFrameCancel,
  requestTimeout: TimeoutRequest = defaultTimeoutRequest,
  cancelTimeout: TimeoutCancel = defaultTimeoutCancel,
) {
  let frameId = 0;
  let timeoutId: number | null = null;
  let scheduleVersion = 0;

  return {
    schedule() {
      scheduleVersion += 1;
      const currentVersion = scheduleVersion;
      run();

      if (frameId) {
        cancelFrame(frameId);
      }
      frameId = requestFrame(() => {
        if (scheduleVersion !== currentVersion) return;
        frameId = 0;
        run();
      });

      if (timeoutId) {
        cancelTimeout(timeoutId);
      }
      timeoutId = requestTimeout(() => {
        if (scheduleVersion !== currentVersion) return;
        timeoutId = null;
        run();
      }, settleDelayMs);
    },
    cancel() {
      scheduleVersion += 1;
      if (frameId) {
        cancelFrame(frameId);
        frameId = 0;
      }
      if (!timeoutId) return;
      cancelTimeout(timeoutId);
      timeoutId = null;
    },
  };
}
