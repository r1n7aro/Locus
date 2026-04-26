<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { t } from "../i18n";
import {
  activateUnityEmbedForInput,
  getUnityEmbedFocusDebugSnapshot,
  setUnityEmbedMouseActivationSuppressed,
  type UnityEmbedFocusDebugSnapshot,
} from "../services/unity";
import ChatWorkspaceView from "./ChatWorkspaceView.vue";
import TopBannerHost from "./TopBannerHost.vue";

withDefaults(defineProps<{
  bootstrapped?: boolean;
  bootstrapError?: string | null;
}>(), {
  bootstrapped: false,
  bootstrapError: null,
});

const ACTIVATION_ALLOWED_SELECTOR = [
  "input",
  "textarea",
  "select",
  "[contenteditable='true']",
  "[contenteditable='']",
  ".chat-composer-input",
].join(",");

let lastActivationSuppressed: boolean | null = null;
let activationErrorLogged = false;
let inputActivationErrorLogged = false;
let focusOutFrame = 0;
let focusDebugSequence = 0;

function focusDebugEnabled(): boolean {
  try {
    return window.localStorage.getItem("locusUnityEmbedFocusDebug") === "1";
  } catch {
    return false;
  }
}

function elementFromTarget(target: EventTarget | null): Element | null {
  return target instanceof Element ? target : null;
}

function describeTarget(target: EventTarget | null): string {
  const element = elementFromTarget(target);
  if (!element) return "";
  const semantic = element.closest(
    ".md-unity-scene-object-ref,.md-unity-asset-ref,.asset-chip,.chat-composer-input,.chat-input-shell",
  );
  const targetElement = semantic ?? element;
  const classes = targetElement instanceof HTMLElement
    ? Array.from(targetElement.classList).slice(0, 4).join(".")
    : "";
  return `${targetElement.tagName.toLowerCase()}${classes ? "." + classes : ""}`;
}

function targetAllowsActivation(target: EventTarget | null): boolean {
  const element = elementFromTarget(target);
  return !!element?.closest(ACTIVATION_ALLOWED_SELECTOR);
}

function focusableInputFromTarget(target: EventTarget | null): HTMLElement | null {
  const element = elementFromTarget(target);
  if (!element) return null;
  const direct = element.closest(ACTIVATION_ALLOWED_SELECTOR);
  if (direct instanceof HTMLElement) return direct;
  return null;
}

function printFocusDebug(
  eventName: string,
  target: EventTarget | null = null,
  extra: Record<string, unknown> = {},
) {
  if (!focusDebugEnabled()) return;
  const seq = ++focusDebugSequence;
  const targetLabel = describeTarget(target);
  getUnityEmbedFocusDebugSnapshot()
    .then((snapshot: UnityEmbedFocusDebugSnapshot | null) => {
      console.info("[Locus][UnityEmbedFocus]", {
        seq,
        event: eventName,
        target: targetLabel,
        ...extra,
        snapshot,
      });
    })
    .catch((error: unknown) => {
      console.warn("[Locus][UnityEmbedFocus] snapshot failed", {
        seq,
        event: eventName,
        target: targetLabel,
        error,
      });
    });
}

function applyMouseActivationSuppressed(suppressed: boolean) {
  if (lastActivationSuppressed === suppressed) return;
  lastActivationSuppressed = suppressed;
  setUnityEmbedMouseActivationSuppressed(suppressed)
    .then(() => printFocusDebug("activation-policy", null, { suppressed }))
    .catch((error: unknown) => {
      if (activationErrorLogged) return;
      activationErrorLogged = true;
      console.warn("[Locus] failed to update Unity embed activation policy:", error);
    });
}

function updateMouseActivationFromTarget(target: EventTarget | null) {
  applyMouseActivationSuppressed(!targetAllowsActivation(target));
}

function activateInputTarget(target: EventTarget | null) {
  const input = focusableInputFromTarget(target);
  if (!input) {
    applyMouseActivationSuppressed(true);
    return;
  }

  lastActivationSuppressed = false;
  activateUnityEmbedForInput()
    .then(() => {
      input.focus({ preventScroll: true });
      printFocusDebug("input-activation", input);
    })
    .catch((error: unknown) => {
      if (inputActivationErrorLogged) return;
      inputActivationErrorLogged = true;
      console.warn("[Locus] failed to activate Unity embed input:", error);
    });
}

function handlePointerDown(event: PointerEvent) {
  activateInputTarget(event.target);
  printFocusDebug("pointerdown", event.target, {
    allowsActivation: targetAllowsActivation(event.target),
    documentHasFocus: document.hasFocus(),
  });
  window.setTimeout(() => {
    printFocusDebug("pointerdown+120ms", event.target, {
      allowsActivation: targetAllowsActivation(event.target),
      documentHasFocus: document.hasFocus(),
    });
  }, 120);
}

function handleClick(event: MouseEvent) {
  printFocusDebug("click", event.target, {
    allowsActivation: targetAllowsActivation(event.target),
    documentHasFocus: document.hasFocus(),
  });
  window.setTimeout(() => {
    printFocusDebug("click+240ms", event.target, {
      allowsActivation: targetAllowsActivation(event.target),
      documentHasFocus: document.hasFocus(),
    });
  }, 240);
}

function handleFocusIn(event: FocusEvent) {
  if (targetAllowsActivation(event.target)) {
    lastActivationSuppressed = false;
  } else {
    updateMouseActivationFromTarget(event.target);
  }
  printFocusDebug("focusin", event.target, {
    allowsActivation: targetAllowsActivation(event.target),
  });
}

function handleFocusOut() {
  printFocusDebug("focusout", document.activeElement);
  if (focusOutFrame) cancelAnimationFrame(focusOutFrame);
  focusOutFrame = requestAnimationFrame(() => {
    focusOutFrame = 0;
    if (!targetAllowsActivation(document.activeElement)) {
      applyMouseActivationSuppressed(true);
    }
  });
}

function handleWindowFocus() {
  printFocusDebug("window-focus", document.activeElement, {
    documentHasFocus: document.hasFocus(),
  });
}

function handleWindowBlur() {
  applyMouseActivationSuppressed(true);
  printFocusDebug("window-blur", document.activeElement, {
    documentHasFocus: document.hasFocus(),
  });
}

onMounted(() => {
  applyMouseActivationSuppressed(true);
  window.addEventListener("focus", handleWindowFocus);
  window.addEventListener("blur", handleWindowBlur);
  printFocusDebug("mounted", document.activeElement);
});

onUnmounted(() => {
  window.removeEventListener("focus", handleWindowFocus);
  window.removeEventListener("blur", handleWindowBlur);
  if (focusOutFrame) cancelAnimationFrame(focusOutFrame);
  focusOutFrame = 0;
  applyMouseActivationSuppressed(true);
});
</script>

<template>
  <main
    class="unity-session-view"
    @pointerdown.capture="handlePointerDown"
    @click.capture="handleClick"
    @focusin.capture="handleFocusIn"
    @focusout.capture="handleFocusOut"
  >
    <TopBannerHost />

    <div v-if="bootstrapError" class="unity-session-state is-error">
      {{ bootstrapError }}
    </div>
    <div v-else-if="!bootstrapped" class="unity-session-state">
      {{ t("common.loading") }}
    </div>
    <ChatWorkspaceView
      v-else
      class="unity-session-workspace"
      active
      layout-mode="auto"
      session-panel-storage-scope="unity"
    />
  </main>
</template>

<style scoped>
.unity-session-view {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--bg-color);
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--border-color) 82%, var(--text-secondary) 18%);
  color: var(--text-color);
}

.unity-session-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  background: var(--panel-bg);
  color: var(--text-secondary);
  font-size: 13px;
  line-height: 1.5;
  text-align: center;
}

.unity-session-state.is-error {
  color: var(--status-danger-fg);
}

.unity-session-workspace {
  flex: 1;
  min-width: 0;
  min-height: 0;
}

:deep(.top-banner-host) {
  top: 10px;
}
</style>
