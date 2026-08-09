<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  ref,
  watch,
} from "vue";
import type { ChatMessage, SessionTurnPreview } from "../../types";
import { t } from "../../i18n";
import {
  buildChatTurnNavigationItems,
  findActiveChatTurnIds,
  normalizeChatTurnPreview,
  type ChatTurnNavigationAnchor,
  type ChatTurnNavigationItem,
} from "./chatTurnNavigation";

const props = defineProps<{
  messages: ChatMessage[];
  sessionId: string | null;
  userMessageIds: string[];
  scrollElement: HTMLElement | null;
  loadPreview?: (messageId: string) => Promise<SessionTurnPreview | null>;
  loadTurn?: (messageId: string) => Promise<boolean>;
}>();

const emit = defineEmits<{
  (event: "navigate"): void;
  (event: "revealState", active: boolean, messageId: string): void;
}>();

const MIN_LEFT_GUTTER = 48;
const VIEWPORT_TOP_OFFSET = 16;
const PREVIEW_OPEN_DELAY_MS = 150;
const PREVIEW_CLOSE_DELAY_MS = 80;
const TARGET_REVEAL_MAX_SETTLE_FRAMES = 30;
const TARGET_REVEAL_STABLE_FRAMES = 6;

const railListRef = ref<HTMLElement | null>(null);
const tooltipRef = ref<HTMLElement | null>(null);
const hasLeftGutter = ref(false);
const listOverflows = ref(false);
const activeIds = ref(new Set<string>());
const previewItemId = ref<string | null>(null);
const scrubTargetId = ref<string | null>(null);
const isScrubbing = ref(false);
const previewOpen = ref(false);
const tooltipStyle = ref<Record<string, string>>({ visibility: "hidden" });
const loadedPreviews = ref(new Map<string, ChatTurnNavigationItem>());
const loadingPreviewIds = ref(new Set<string>());
const failedPreviewIds = ref(new Set<string>());

const items = computed(() => buildChatTurnNavigationItems(props.messages, props.userMessageIds));
const itemKey = computed(() => items.value.map((item) => item.id).join("\u241e"));
const visibleItems = computed(() => items.value);
const previewItem = computed(() => {
  const item = visibleItems.value.find((candidate) => candidate.id === previewItemId.value) ?? null;
  return item ? loadedPreviews.value.get(item.id) ?? item : null;
});
const previewLoading = computed(() => (
  !!previewItemId.value && loadingPreviewIds.value.has(previewItemId.value)
));
const previewFailed = computed(() => (
  !!previewItemId.value && failedPreviewIds.value.has(previewItemId.value)
));
const canShow = computed(() =>
  hasLeftGutter.value && visibleItems.value.length > 0);

let connectedScrollElement: HTMLElement | null = null;
let contentResizeObserver: ResizeObserver | null = null;
let transcriptMutationObserver: MutationObserver | null = null;
let layoutFrame = 0;
let tooltipFrame = 0;
let hoverTimer: number | null = null;
let closeTimer: number | null = null;
let anchorPositions: ChatTurnNavigationAnchor[] = [];
let targetElements = new Map<string, HTMLElement>();
let anchorButton: HTMLButtonElement | null = null;
let pointerInsideRail = false;
let pointerInsideTooltip = false;
let suppressNextClick = false;
let scrubState: {
  pointerId: number;
  itemId: string;
  moved: boolean;
} | null = null;

function requestFrame(callback: FrameRequestCallback) {
  return window.requestAnimationFrame(callback);
}

function cancelFrame(frame: number) {
  if (frame) window.cancelAnimationFrame(frame);
}

function cssEscape(value: string) {
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(value);
  }
  return value.replace(/["\\]/g, "\\$&");
}

function findTarget(scrollElement: HTMLElement, itemId: string) {
  return scrollElement.querySelector<HTMLElement>(
    `.chat-transcript-item-stack[data-chat-message-role="user"][data-chat-message-id="${cssEscape(itemId)}"]`,
  );
}

function replaceSet(target: typeof activeIds, values: Iterable<string>) {
  const next = new Set(values);
  const current = target.value;
  if (current.size === next.size && [...current].every((value) => next.has(value))) return;
  target.value = next;
}

function updateActiveItems() {
  const scrollElement = connectedScrollElement;
  if (!scrollElement || anchorPositions.length === 0) {
    replaceSet(activeIds, []);
    return;
  }

  const viewportTop = scrollElement.scrollTop + VIEWPORT_TOP_OFFSET;
  const viewportBottom = scrollElement.scrollTop + scrollElement.clientHeight;
  replaceSet(
    activeIds,
    findActiveChatTurnIds(
      anchorPositions,
      viewportTop,
      viewportBottom,
      Math.max(scrollElement.scrollHeight, viewportBottom),
    ),
  );
}

function measureLayout() {
  layoutFrame = 0;
  const scrollElement = connectedScrollElement;
  if (!scrollElement) {
    targetElements = new Map();
    anchorPositions = [];
    hasLeftGutter.value = false;
    listOverflows.value = false;
    return;
  }

  const scrollRect = scrollElement.getBoundingClientRect();
  const nextTargets = new Map<string, HTMLElement>();
  const nextAnchors: ChatTurnNavigationAnchor[] = [];

  for (const item of items.value) {
    const target = findTarget(scrollElement, item.id);
    if (!target) continue;
    const targetRect = target.getBoundingClientRect();
    nextTargets.set(item.id, target);
    nextAnchors.push({
      id: item.id,
      start: scrollElement.scrollTop + targetRect.top - scrollRect.top,
    });
  }

  const firstTarget = nextTargets.values().next().value as HTMLElement | undefined;
  const contentColumn = firstTarget?.closest<HTMLElement>(".chat-transcript-message-content")
    ?? scrollElement.querySelector<HTMLElement>(".chat-transcript-message-content")
    ?? firstTarget;
  const contentLeft = contentColumn?.getBoundingClientRect().left ?? scrollRect.left;

  targetElements = nextTargets;
  anchorPositions = nextAnchors;
  hasLeftGutter.value = contentLeft - scrollRect.left >= MIN_LEFT_GUTTER;
  updateActiveItems();
  scheduleTooltipPosition();
  void nextTick(updateListOverflow);
}

function updateListOverflow() {
  const list = railListRef.value;
  listOverflows.value = !!list && list.scrollHeight > list.clientHeight + 1;
}

function scheduleLayoutMeasure() {
  if (layoutFrame || typeof window === "undefined") return;
  layoutFrame = requestFrame(measureLayout);
}

function positionTooltip() {
  tooltipFrame = 0;
  const button = anchorButton;
  const tooltip = tooltipRef.value;
  if (!button || !tooltip || !previewOpen.value || !previewItem.value) return;

  const buttonRect = button.getBoundingClientRect();
  const viewportWidth = Math.max(1, window.innerWidth);
  const viewportHeight = Math.max(1, window.innerHeight);
  const width = Math.min(320, viewportWidth - 16);
  const height = tooltip.offsetHeight;
  const left = Math.max(8, Math.min(buttonRect.right, viewportWidth - width - 8));
  const top = Math.max(
    8,
    Math.min(buttonRect.top + buttonRect.height / 2 - height / 2, viewportHeight - height - 8),
  );

  tooltipStyle.value = {
    left: `${Math.round(left)}px`,
    top: `${Math.round(top)}px`,
    width: `${Math.round(width)}px`,
  };
}

function scheduleTooltipPosition() {
  if (tooltipFrame || typeof window === "undefined") return;
  tooltipFrame = requestFrame(positionTooltip);
}

function clearHoverTimer() {
  if (hoverTimer === null) return;
  window.clearTimeout(hoverTimer);
  hoverTimer = null;
}

function clearCloseTimer() {
  if (closeTimer === null) return;
  window.clearTimeout(closeTimer);
  closeTimer = null;
}

function closePreview() {
  clearHoverTimer();
  clearCloseTimer();
  previewOpen.value = false;
  previewItemId.value = null;
  anchorButton = null;
  tooltipStyle.value = { visibility: "hidden" };
}

function schedulePreviewClose() {
  clearCloseTimer();
  closeTimer = window.setTimeout(() => {
    closeTimer = null;
    if (!pointerInsideRail && !pointerInsideTooltip && scrubState === null) {
      closePreview();
    }
  }, PREVIEW_CLOSE_DELAY_MS);
}

function showPreview(item: ChatTurnNavigationItem, button: HTMLButtonElement) {
  clearHoverTimer();
  clearCloseTimer();
  previewItemId.value = item.id;
  anchorButton = button;
  previewOpen.value = true;
  tooltipStyle.value = { visibility: "hidden" };
  void nextTick(scheduleTooltipPosition);
  if (!scrubState || !scrubState.moved) {
    void ensurePreviewLoaded(item);
  }
}

async function ensurePreviewLoaded(item: ChatTurnNavigationItem) {
  if (!item.deferred || loadedPreviews.value.has(item.id) || loadingPreviewIds.value.has(item.id)) {
    return;
  }
  if (!props.loadPreview) return;
  loadingPreviewIds.value = new Set(loadingPreviewIds.value).add(item.id);
  const nextFailed = new Set(failedPreviewIds.value);
  nextFailed.delete(item.id);
  failedPreviewIds.value = nextFailed;
  try {
    const preview = await props.loadPreview(item.id);
    if (!preview || preview.messageId !== item.id) {
      failedPreviewIds.value = new Set(failedPreviewIds.value).add(item.id);
      return;
    }
    const nextPreviews = new Map(loadedPreviews.value);
    nextPreviews.set(item.id, normalizeChatTurnPreview(preview));
    loadedPreviews.value = nextPreviews;
    if (previewItemId.value === item.id) {
      void nextTick(scheduleTooltipPosition);
    }
  } catch {
    failedPreviewIds.value = new Set(failedPreviewIds.value).add(item.id);
  } finally {
    const nextLoading = new Set(loadingPreviewIds.value);
    nextLoading.delete(item.id);
    loadingPreviewIds.value = nextLoading;
  }
}

function schedulePreview(item: ChatTurnNavigationItem, button: HTMLButtonElement) {
  if (scrubState) {
    showPreview(item, button);
    return;
  }
  clearHoverTimer();
  hoverTimer = window.setTimeout(() => {
    hoverTimer = null;
    showPreview(item, button);
  }, PREVIEW_OPEN_DELAY_MS);
}

function prefersReducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

function flashTarget(target: HTMLElement) {
  if (prefersReducedMotion()) return;
  const bubble = target.querySelector<HTMLElement>(".chat-transcript-plain-text") ?? target;
  const originalColor = window.getComputedStyle(bubble).backgroundColor;
  bubble.animate?.(
    [
      { backgroundColor: "color-mix(in srgb, var(--text-color) 14%, var(--msg-user-bg))" },
      {
        backgroundColor: "color-mix(in srgb, var(--text-color) 14%, var(--msg-user-bg))",
        offset: 0.35,
      },
      { backgroundColor: originalColor },
    ],
    {
      duration: 1400,
      easing: "cubic-bezier(0.23, 1, 0.32, 1)",
    },
  );
}

async function revealItem(
  item: ChatTurnNavigationItem,
  behavior: ScrollBehavior,
  loadMissing = true,
) {
  const scrollElement = connectedScrollElement;
  const revealGuardActive = loadMissing;
  let target = targetElements.get(item.id)
    ?? (scrollElement ? findTarget(scrollElement, item.id) : null);
  if (revealGuardActive) {
    emit("revealState", true, item.id);
  }
  try {
    if (!target && loadMissing && props.loadTurn) {
      emit("navigate");
      const loaded = await props.loadTurn(item.id);
      if (!loaded) return;
      await nextTick();
      measureLayout();
      target = targetElements.get(item.id)
        ?? (scrollElement ? findTarget(scrollElement, item.id) : null);
    }
    if (!target) return;

    emit("navigate");
    if (revealGuardActive) {
      await revealTargetSettled(target);
    } else {
      target.scrollIntoView({
        behavior: prefersReducedMotion() ? "auto" : behavior,
        block: "start",
        inline: "nearest",
      });
    }
    flashTarget(target);
  } finally {
    if (revealGuardActive) emit("revealState", false, item.id);
  }
}

async function revealTargetSettled(target: HTMLElement) {
  const scrollElement = connectedScrollElement;
  if (!scrollElement) return;
  let stableFrames = 0;
  for (let frame = 0; frame < TARGET_REVEAL_MAX_SETTLE_FRAMES; frame += 1) {
    target.scrollIntoView({ behavior: "auto", block: "start", inline: "nearest" });
    await new Promise<void>((resolve) => requestFrame(() => resolve()));
    const offset = target.getBoundingClientRect().top - scrollElement.getBoundingClientRect().top;
    const maxScrollTop = Math.max(0, scrollElement.scrollHeight - scrollElement.clientHeight);
    const settled = Math.abs(offset) <= 1 || (
      Math.abs(scrollElement.scrollTop - maxScrollTop) <= 1 && offset >= -1
    );
    stableFrames = settled ? stableFrames + 1 : 0;
    if (stableFrames >= TARGET_REVEAL_STABLE_FRAMES) break;
  }
}

function itemForButton(button: Element | null) {
  if (!(button instanceof HTMLButtonElement)) return null;
  const itemId = button.dataset.turnNavigationItemId;
  if (!itemId) return null;
  const item = visibleItems.value.find((candidate) => candidate.id === itemId);
  return item ? { button, item } : null;
}

function itemAtPointer(clientY: number) {
  const list = railListRef.value;
  if (!list) return null;
  const listRect = list.getBoundingClientRect();
  const clampedY = Math.max(listRect.top, Math.min(clientY, listRect.bottom - 1));
  const hit = document.elementFromPoint(listRect.left + listRect.width / 2, clampedY)
    ?.closest("[data-turn-navigation-item-id]") ?? null;
  const direct = itemForButton(hit);
  if (direct) return direct;

  let nearest: { button: HTMLButtonElement; item: ChatTurnNavigationItem } | null = null;
  let nearestDistance = Number.POSITIVE_INFINITY;
  for (const button of list.querySelectorAll<HTMLButtonElement>("[data-turn-navigation-item-id]")) {
    const candidate = itemForButton(button);
    if (!candidate) continue;
    const rect = button.getBoundingClientRect();
    const distance = Math.abs(clientY - (rect.top + rect.height / 2));
    if (distance < nearestDistance) {
      nearest = candidate;
      nearestDistance = distance;
    }
  }
  return nearest;
}

function handleButtonPointerEnter(
  item: ChatTurnNavigationItem,
  event: PointerEvent,
) {
  pointerInsideRail = true;
  schedulePreview(item, event.currentTarget as HTMLButtonElement);
}

function handleButtonFocus(item: ChatTurnNavigationItem, event: FocusEvent) {
  showPreview(item, event.currentTarget as HTMLButtonElement);
}

function handleButtonBlur() {
  window.setTimeout(() => {
    const list = railListRef.value;
    if (list?.contains(document.activeElement)) return;
    schedulePreviewClose();
  }, 0);
}

function handleButtonClick(item: ChatTurnNavigationItem) {
  if (suppressNextClick) {
    suppressNextClick = false;
    return;
  }
  void revealItem(item, "smooth");
}

function handleRailPointerEnter() {
  pointerInsideRail = true;
  clearCloseTimer();
}

function handleRailPointerLeave() {
  pointerInsideRail = false;
  if (!scrubState) scrubTargetId.value = null;
  schedulePreviewClose();
}

function handlePointerDown(event: PointerEvent) {
  if (event.button !== 0) return;
  const list = railListRef.value;
  const targetElement = (event.target as Element | null)
    ?.closest("[data-turn-navigation-item-id]") ?? null;
  const target = itemForButton(targetElement);
  if (!list || !target) return;

  event.preventDefault();
  target.button.focus({ preventScroll: true });
  list.setPointerCapture?.(event.pointerId);
  scrubState = {
    pointerId: event.pointerId,
    itemId: target.item.id,
    moved: false,
  };
  isScrubbing.value = true;
  scrubTargetId.value = target.item.id;
  showPreview(target.item, target.button);
}

function handlePointerMove(event: PointerEvent) {
  const state = scrubState;
  if (!state || state.pointerId !== event.pointerId) return;
  if (event.buttons % 2 === 0) {
    handlePointerEnd(event);
    return;
  }

  const target = itemAtPointer(event.clientY);
  if (!target || target.item.id === state.itemId) return;
  state.itemId = target.item.id;
  state.moved = true;
  scrubTargetId.value = target.item.id;
  showPreview(target.item, target.button);
  void revealItem(target.item, "auto", false);
}

function handlePointerEnd(event: PointerEvent) {
  const state = scrubState;
  if (!state || state.pointerId !== event.pointerId) return;
  const list = railListRef.value;
  const finalItem = visibleItems.value.find((item) => item.id === state.itemId);
  if (finalItem) void revealItem(finalItem, state.moved ? "auto" : "smooth");
  suppressNextClick = true;
  window.setTimeout(() => {
    suppressNextClick = false;
  }, 0);
  scrubState = null;
  isScrubbing.value = false;
  scrubTargetId.value = null;
  if (finalItem) void ensurePreviewLoaded(finalItem);
  if (list?.hasPointerCapture?.(event.pointerId)) {
    list.releasePointerCapture?.(event.pointerId);
  }
}

function handleTooltipPointerEnter() {
  pointerInsideTooltip = true;
  clearCloseTimer();
}

function handleTooltipPointerLeave() {
  pointerInsideTooltip = false;
  schedulePreviewClose();
}

function handleTranscriptScroll() {
  updateActiveItems();
  scheduleTooltipPosition();
}

function disconnect() {
  cancelFrame(layoutFrame);
  cancelFrame(tooltipFrame);
  layoutFrame = 0;
  tooltipFrame = 0;
  contentResizeObserver?.disconnect();
  contentResizeObserver = null;
  transcriptMutationObserver?.disconnect();
  transcriptMutationObserver = null;
  connectedScrollElement?.removeEventListener("scroll", handleTranscriptScroll);
  window.removeEventListener("resize", scheduleLayoutMeasure);
  connectedScrollElement = null;
}

function connect(scrollElement: HTMLElement | null) {
  if (connectedScrollElement === scrollElement) {
    scheduleLayoutMeasure();
    return;
  }
  disconnect();
  connectedScrollElement = scrollElement;
  if (!scrollElement) {
    scheduleLayoutMeasure();
    return;
  }

  scrollElement.addEventListener("scroll", handleTranscriptScroll, { passive: true });
  window.addEventListener("resize", scheduleLayoutMeasure, { passive: true });

  if (typeof ResizeObserver !== "undefined") {
    contentResizeObserver = new ResizeObserver(scheduleLayoutMeasure);
    contentResizeObserver.observe(scrollElement);
    const content = scrollElement.querySelector<HTMLElement>(".chat-transcript-content");
    if (content) contentResizeObserver.observe(content);
  }

  if (typeof MutationObserver !== "undefined") {
    transcriptMutationObserver = new MutationObserver(scheduleLayoutMeasure);
    transcriptMutationObserver.observe(scrollElement, { childList: true, subtree: true });
  }

  scheduleLayoutMeasure();
}

watch(
  () => [props.scrollElement, itemKey.value] as const,
  async ([scrollElement]) => {
    await nextTick();
    connect(scrollElement);
  },
  { immediate: true, flush: "post" },
);

watch(
  () => props.sessionId,
  () => {
    loadedPreviews.value = new Map();
    loadingPreviewIds.value = new Set();
    failedPreviewIds.value = new Set();
    closePreview();
  },
);

watch(canShow, (show) => {
  if (!show) {
    listOverflows.value = false;
    closePreview();
    return;
  }
  void nextTick(updateListOverflow);
});

onBeforeUnmount(() => {
  disconnect();
  clearHoverTimer();
  clearCloseTimer();
});
</script>

<template>
  <nav
    v-if="canShow"
    class="chat-turn-navigation-rail"
    :aria-label="t('chat.turnNavigation.ariaLabel')"
  >
    <div
      ref="railListRef"
      class="chat-turn-navigation-list"
      :class="{ 'is-overflowing': listOverflows }"
      :data-scrubbing="isScrubbing ? '' : undefined"
      @pointerenter="handleRailPointerEnter"
      @pointerleave="handleRailPointerLeave"
      @pointerdown="handlePointerDown"
      @pointermove="handlePointerMove"
      @pointerup="handlePointerEnd"
      @pointercancel="handlePointerEnd"
      @lostpointercapture="handlePointerEnd"
    >
      <button
        v-for="(item, index) in visibleItems"
        :key="item.id"
        type="button"
        class="chat-turn-navigation-button"
        :data-turn-navigation-item-id="item.id"
        :data-scrub-target="scrubTargetId === item.id ? '' : undefined"
        :aria-current="activeIds.has(item.id) ? 'true' : undefined"
        :aria-describedby="previewOpen && previewItemId === item.id ? 'chat-turn-navigation-tooltip' : undefined"
        :aria-label="t('chat.turnNavigation.jumpTo', index + 1)"
        @pointerenter="handleButtonPointerEnter(item, $event)"
        @focus="handleButtonFocus(item, $event)"
        @blur="handleButtonBlur"
        @click="handleButtonClick(item)"
      >
        <span class="chat-turn-navigation-marker-frame">
          <span class="chat-turn-navigation-marker"></span>
        </span>
      </button>
    </div>

    <Teleport to="body">
      <Transition name="chat-turn-navigation-tooltip">
        <div
          v-if="previewOpen && previewItem"
          id="chat-turn-navigation-tooltip"
          ref="tooltipRef"
          role="tooltip"
          class="chat-turn-navigation-tooltip"
          :style="tooltipStyle"
          @pointerenter="handleTooltipPointerEnter"
          @pointerleave="handleTooltipPointerLeave"
        >
          <div class="chat-turn-navigation-prompt" :class="{ 'is-status': previewLoading || previewFailed }">
            {{ previewLoading
              ? t("chat.turnNavigation.loading")
              : previewFailed
                ? t("chat.turnNavigation.loadFailed")
                : previewItem.prompt || t("chat.turnNavigation.noContent") }}
          </div>
          <div v-if="!previewLoading && !previewFailed && previewItem.response" class="chat-turn-navigation-response">
            {{ previewItem.response }}
          </div>
        </div>
      </Transition>
    </Teleport>
  </nav>
</template>

<style scoped>
.chat-turn-navigation-rail {
  position: absolute;
  top: 50%;
  left: 12px;
  z-index: 20;
  transform: translateY(-50%);
  animation: chat-turn-navigation-enter 150ms cubic-bezier(0.23, 1, 0.32, 1);
}

.chat-turn-navigation-list {
  display: flex;
  flex-direction: column;
  max-height: min(70vh, 640px);
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-width: none;
}

.chat-turn-navigation-list.is-overflowing {
  mask-image: linear-gradient(
    to bottom,
    transparent,
    #000 20px,
    #000 calc(100% - 20px),
    transparent
  );
}

.chat-turn-navigation-list::-webkit-scrollbar {
  display: none;
}

.chat-turn-navigation-button {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  width: 36px;
  height: 10px;
  padding: 0;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--text-secondary);
  cursor: default;
}

.chat-turn-navigation-marker-frame {
  display: flex;
  align-items: center;
  width: 30px;
  height: 2px;
}

.chat-turn-navigation-marker {
  --marker-progress: 0;
  width: 26px;
  height: 2px;
  background: var(--text-secondary);
  opacity: 0.4;
  transform: scaleX(calc(0.2308 + 0.7692 * var(--marker-progress)));
  transform-origin: left center;
  transition:
    transform 160ms linear(0, 0.398 10%, 0.682 20%, 0.843 30%, 0.925 40%, 0.972 50%, 1.004 60%, 1.008 70%, 1.003 80%, 1),
    background-color 120ms ease,
    opacity 120ms ease;
}

.chat-turn-navigation-button[aria-current="true"] .chat-turn-navigation-marker {
  background: var(--text-color);
  opacity: 0.6;
}

:is(
  .chat-turn-navigation-button:has(+ .chat-turn-navigation-button[data-scrub-target]),
  .chat-turn-navigation-button[data-scrub-target] + .chat-turn-navigation-button
) .chat-turn-navigation-marker {
  --marker-progress: 0.7;
}

:is(
  .chat-turn-navigation-button:has(+ .chat-turn-navigation-button + .chat-turn-navigation-button[data-scrub-target]),
  .chat-turn-navigation-button[data-scrub-target] + .chat-turn-navigation-button + .chat-turn-navigation-button
) .chat-turn-navigation-marker {
  --marker-progress: 0.4;
}

:is(
  .chat-turn-navigation-button:has(+ .chat-turn-navigation-button + .chat-turn-navigation-button + .chat-turn-navigation-button[data-scrub-target]),
  .chat-turn-navigation-button[data-scrub-target] + .chat-turn-navigation-button + .chat-turn-navigation-button + .chat-turn-navigation-button
) .chat-turn-navigation-marker {
  --marker-progress: 0.2;
}

:is(
  .chat-turn-navigation-button:focus-visible,
  .chat-turn-navigation-button[data-scrub-target]
) .chat-turn-navigation-marker {
  --marker-progress: 1;
  background: var(--text-color);
  opacity: 1;
}

.chat-turn-navigation-list[data-scrubbing] .chat-turn-navigation-marker {
  transition-duration: 0s;
}

.chat-turn-navigation-list[data-scrubbing]
  .chat-turn-navigation-button[aria-current="true"]:not([data-scrub-target])
  .chat-turn-navigation-marker {
  background: var(--text-secondary);
  opacity: 0.4;
}

.chat-turn-navigation-tooltip {
  position: fixed;
  z-index: 1000;
  box-sizing: border-box;
  max-width: calc(100vw - 16px);
  overflow: hidden;
  padding: 8px;
  border: 0.5px solid var(--border-color);
  border-radius: 12px;
  background: color-mix(in srgb, var(--panel-bg) 95%, transparent);
  color: var(--text-color);
  box-shadow: 0 10px 28px rgba(15, 17, 21, 0.12);
  backdrop-filter: blur(10px);
  font-size: 13px;
  line-height: 20px;
  pointer-events: auto;
}

:global(:root[data-theme="dark"]) .chat-turn-navigation-tooltip {
  box-shadow: 0 14px 32px rgba(0, 0, 0, 0.34);
}

.chat-turn-navigation-prompt,
.chat-turn-navigation-response {
  display: -webkit-box;
  overflow: hidden;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  -webkit-box-orient: vertical;
}

.chat-turn-navigation-prompt {
  color: var(--text-color);
  font-weight: 600;
  -webkit-line-clamp: 3;
}

.chat-turn-navigation-prompt.is-status {
  color: var(--text-secondary);
  font-weight: 400;
}

.chat-turn-navigation-response {
  margin-top: 4px;
  color: var(--text-secondary);
  -webkit-line-clamp: 3;
}

.chat-turn-navigation-tooltip-enter-active,
.chat-turn-navigation-tooltip-leave-active {
  transition: opacity 120ms ease, transform 120ms cubic-bezier(0.23, 1, 0.32, 1);
}

.chat-turn-navigation-tooltip-enter-from,
.chat-turn-navigation-tooltip-leave-to {
  opacity: 0;
  transform: translateX(-4px);
}

@media (hover: hover) and (pointer: fine) {
  .chat-turn-navigation-list:not([data-scrubbing]):hover
    .chat-turn-navigation-button[aria-current="true"]:not(:hover):not(:focus-visible)
    .chat-turn-navigation-marker {
    background: var(--text-secondary);
    opacity: 0.4;
  }

  .chat-turn-navigation-list:not([data-scrubbing])
    :is(
      .chat-turn-navigation-button:has(+ .chat-turn-navigation-button:hover),
      .chat-turn-navigation-button:hover + .chat-turn-navigation-button
    )
    .chat-turn-navigation-marker {
    --marker-progress: 0.7;
  }

  .chat-turn-navigation-list:not([data-scrubbing])
    :is(
      .chat-turn-navigation-button:has(+ .chat-turn-navigation-button + .chat-turn-navigation-button:hover),
      .chat-turn-navigation-button:hover + .chat-turn-navigation-button + .chat-turn-navigation-button
    )
    .chat-turn-navigation-marker {
    --marker-progress: 0.4;
  }

  .chat-turn-navigation-list:not([data-scrubbing])
    :is(
      .chat-turn-navigation-button:has(+ .chat-turn-navigation-button + .chat-turn-navigation-button + .chat-turn-navigation-button:hover),
      .chat-turn-navigation-button:hover + .chat-turn-navigation-button + .chat-turn-navigation-button + .chat-turn-navigation-button
    )
    .chat-turn-navigation-marker {
    --marker-progress: 0.2;
  }

  .chat-turn-navigation-list:not([data-scrubbing])
    .chat-turn-navigation-button:hover
    .chat-turn-navigation-marker {
    --marker-progress: 1;
    background: var(--text-color);
    opacity: 1;
  }
}

@keyframes chat-turn-navigation-enter {
  from { opacity: 0; }
  to { opacity: 1; }
}

@media (prefers-reduced-motion: reduce) {
  .chat-turn-navigation-rail {
    animation-duration: 0s;
  }

  .chat-turn-navigation-marker,
  .chat-turn-navigation-tooltip-enter-active,
  .chat-turn-navigation-tooltip-leave-active {
    transition-duration: 0s;
  }
}
</style>
