<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide";
import { t } from "../i18n";
import type { CompactedContextOutput } from "../types";
import { normalizeAppError } from "../services/errors";
import { getCompactedContextOutput } from "../services/session";
import {
  CONTEXT_COMPACTION_WINDOW_EVENT,
  CONTEXT_COMPACTION_WINDOW_LABEL,
  getContextCompactionWindowPayload,
  type ContextCompactionWindowPayload,
} from "../services/contextCompactionWindow";
import { getSubWindowClaimedQuery } from "../services/subWindow";
import AssetChip from "./AssetChip.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import MarkdownRenderer from "./MarkdownRenderer.vue";

const appWindow = getCurrentWindow();
const output = ref<CompactedContextOutput | null>(null);
const loading = ref(false);
const error = ref("");
const activeTargetKey = ref("");
let unlistenPayload: UnlistenFn | null = null;
let loadSequence = 0;

const snapshotNotice = computed(() => {
  if (output.value?.snapshotStatus === "reconstructed") {
    return t("chat.compactedContext.reconstructed");
  }
  if (output.value?.snapshotStatus === "partial") {
    return t("chat.compactedContext.partial");
  }
  return "";
});

function targetKey(payload: ContextCompactionWindowPayload) {
  return `${payload.sessionId.trim()}\u0000${payload.messageId.trim()}`;
}

async function loadPayload(payload: ContextCompactionWindowPayload) {
  const sessionId = payload.sessionId.trim();
  const messageId = payload.messageId.trim();
  if (!sessionId || !messageId) {
    output.value = null;
    error.value = t("chat.compactedContext.loadFailed");
    return;
  }

  const nextTargetKey = targetKey({ sessionId, messageId });
  if (nextTargetKey === activeTargetKey.value && output.value && !error.value) return;
  activeTargetKey.value = nextTargetKey;
  const sequence = ++loadSequence;
  loading.value = true;
  error.value = "";
  output.value = null;
  try {
    const result = await getCompactedContextOutput(sessionId, messageId);
    if (sequence !== loadSequence) return;
    output.value = result;
  } catch (cause) {
    if (sequence !== loadSequence) return;
    const normalized = normalizeAppError(cause);
    error.value = normalized.message || t("chat.compactedContext.loadFailed");
  } finally {
    if (sequence === loadSequence) loading.value = false;
  }
}

function markdownInlineCode(value: string) {
  const backtickRuns = value.match(/`+/g) ?? [];
  const fence = "`".repeat(Math.max(0, ...backtickRuns.map((run) => run.length)) + 1);
  const padding = value.startsWith("`") || value.endsWith("`") ? " " : "";
  return `${fence}${padding}${value}${padding}${fence}`;
}

/** Keep the exact restored snippet text while turning each restored path
 * heading into the shared lightweight file-reference treatment. */
function contextMarkdown(content: string) {
  let insideRestoredFiles = false;
  return content
    .split("\n")
    .map((line) => {
      if (line.trim() === "### Restored File Context") {
        insideRestoredFiles = true;
        return line;
      }
      if (!insideRestoredFiles || !line.startsWith("#### ")) return line;
      const path = line.slice(5).trim();
      return path ? `#### ${markdownInlineCode(path)}` : line;
    })
    .join("\n");
}

function imageDataUrl(image: { mimeType: string; data: string }) {
  return `data:${image.mimeType};base64,${image.data}`;
}

function roleLabel(role: "user" | "assistant" | "tool") {
  if (role === "user") return t("chat.compactedContext.roleUser");
  if (role === "assistant") return t("chat.compactedContext.roleAssistant");
  return t("chat.compactedContext.roleTool");
}

async function closeWindow() {
  try {
    await appWindow.close();
    return;
  } catch {
    // fall through
  }
  await appWindow.destroy().catch(() => {});
}

onMounted(async () => {
  void loadPayload(getContextCompactionWindowPayload());
  let payloadEventSeen = false;
  try {
    unlistenPayload = await listen<ContextCompactionWindowPayload>(
      CONTEXT_COMPACTION_WINDOW_EVENT,
      (event) => {
        payloadEventSeen = true;
        void loadPayload(event.payload);
      },
    );
    const claimedQuery = await getSubWindowClaimedQuery(CONTEXT_COMPACTION_WINDOW_LABEL)
      .catch(() => null);
    if (claimedQuery && !payloadEventSeen) {
      void loadPayload(getContextCompactionWindowPayload(`?${claimedQuery}`));
    }
  } catch {
    // The initial URL remains usable when event hooks are unavailable.
  }
});

onUnmounted(() => {
  unlistenPayload?.();
  unlistenPayload = null;
  loadSequence += 1;
});
</script>

<template>
  <div class="compacted-context-window-root">
    <div class="compacted-context-titlebar">
      <span class="compacted-context-title">{{ t("chat.compactedContext.windowTitle") }}</span>
      <button
        type="button"
        class="compacted-context-close"
        :title="t('app.win.close')"
        @click="closeWindow"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <main class="compacted-context-body">
      <div v-if="error" class="compacted-context-state is-error">{{ error }}</div>
      <div v-else-if="loading && !output" class="compacted-context-state">
        {{ t("common.loading") }}
      </div>
      <template v-else-if="output">
        <div v-if="snapshotNotice" class="compacted-context-notice">{{ snapshotNotice }}</div>

        <section class="compacted-context-entry is-placeholder">
          <div class="compacted-context-role">{{ t("chat.compactedContext.roleSystem") }}</div>
          <div class="compacted-context-entry-body">
            <span class="compacted-context-placeholder">
              {{ t("chat.compactedContext.systemPlaceholder") }}
            </span>
          </div>
        </section>

        <section
          v-for="message in output.messages"
          :key="message.id"
          class="compacted-context-entry"
        >
          <div class="compacted-context-role">{{ roleLabel(message.role) }}</div>
          <div class="compacted-context-entry-body">
            <div
              v-if="message.promptPrefixPlaceholder"
              class="compacted-context-injection-placeholder"
            >
              {{ t("chat.compactedContext.promptPrefixPlaceholder") }}
            </div>

            <template v-if="output.compactionKind === 'codexEncrypted' && message.id === output.messageId">
              <div class="compacted-context-encrypted">
                <div class="compacted-context-encrypted-title">
                  {{ t("chat.compactedContext.codexEncrypted") }}
                </div>
                <div class="compacted-context-encrypted-detail">
                  {{ t("chat.compactedContext.codexEncryptedDetail", output.encryptedContentChars || 0) }}
                </div>
              </div>
              <details class="compacted-context-local-fallback">
                <summary>{{ t("chat.compactedContext.localFallback") }}</summary>
                <MarkdownRenderer
                  class="compacted-context-markdown"
                  :content="contextMarkdown(message.content)"
                  enable-file-refs
                />
              </details>
            </template>
            <MarkdownRenderer
              v-else
              class="compacted-context-markdown"
              :content="contextMarkdown(message.content)"
              enable-file-refs
            />

            <div v-if="message.assetRefs?.length" class="compacted-context-asset-refs">
              <AssetChip
                v-for="assetRef in message.assetRefs"
                :key="`${assetRef.kind}:${assetRef.path}`"
                :path="assetRef.path"
                :kind="assetRef.kind"
                context-menu-mode="inherit"
              />
            </div>
            <div v-if="message.images?.length" class="compacted-context-images">
              <img
                v-for="(image, index) in message.images"
                :key="index"
                :src="imageDataUrl(image)"
                class="compacted-context-image"
                :alt="t('chat.compactedContext.imageAlt', index + 1)"
              />
            </div>

            <div
              v-if="message.promptSuffixPlaceholder"
              class="compacted-context-injection-placeholder"
            >
              {{ t("chat.compactedContext.promptSuffixPlaceholder") }}
            </div>
          </div>
        </section>
      </template>
    </main>
  </div>
</template>

<style scoped>
.compacted-context-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
}

.compacted-context-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.compacted-context-title {
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.compacted-context-close {
  -webkit-app-region: no-drag;
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.compacted-context-close:hover,
.compacted-context-close:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
  outline: none;
}

.compacted-context-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  background: var(--panel-bg);
}

.compacted-context-state {
  min-height: 160px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  color: var(--text-secondary);
  font-size: 13px;
}

.compacted-context-state.is-error {
  color: var(--status-danger-fg);
}

.compacted-context-notice {
  padding: 8px 18px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  background: var(--sidebar-bg);
  font-size: 11px;
  line-height: 1.5;
}

.compacted-context-entry {
  display: grid;
  grid-template-columns: 92px minmax(0, 1fr);
  border-bottom: 1px solid var(--border-color);
}

.compacted-context-role {
  padding: 16px 12px 16px 18px;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--sidebar-bg) 70%, var(--panel-bg) 30%);
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
}

.compacted-context-entry-body {
  min-width: 0;
  padding: 14px 20px 18px;
}

.compacted-context-placeholder,
.compacted-context-injection-placeholder {
  display: inline-block;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.compacted-context-injection-placeholder {
  display: block;
  margin: 2px 0 12px;
  padding: 7px 9px;
  border-left: 2px solid var(--border-strong);
  background: color-mix(in srgb, var(--sidebar-bg) 56%, transparent);
}

.compacted-context-injection-placeholder:last-child {
  margin: 12px 0 2px;
}

.compacted-context-markdown :deep(.markdown-body) {
  max-width: 920px;
  margin: 0;
  font-size: 13px;
  line-height: 1.68;
}

.compacted-context-encrypted {
  padding: 9px 0 12px;
}

.compacted-context-encrypted-title {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 600;
}

.compacted-context-encrypted-detail {
  margin-top: 4px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.55;
}

.compacted-context-local-fallback {
  margin-top: 4px;
  border-top: 1px solid var(--border-color);
  padding-top: 10px;
}

.compacted-context-local-fallback > summary {
  width: fit-content;
  color: var(--text-secondary);
  font-size: 12px;
  cursor: pointer;
  user-select: none;
}

.compacted-context-local-fallback[open] > summary {
  margin-bottom: 12px;
  color: var(--text-color);
}

.compacted-context-asset-refs {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 12px;
}

.compacted-context-images {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 12px;
}

.compacted-context-image {
  display: block;
  width: auto;
  max-width: min(100%, 520px);
  max-height: 360px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  object-fit: contain;
}

@media (max-width: 680px) {
  .compacted-context-entry {
    grid-template-columns: 72px minmax(0, 1fr);
  }

  .compacted-context-role {
    padding-left: 12px;
  }

  .compacted-context-entry-body {
    padding-left: 14px;
    padding-right: 14px;
  }
}
</style>
