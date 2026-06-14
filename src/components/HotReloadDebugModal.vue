<script setup lang="ts">
import { t } from "../i18n";
import BaseButton from "./ui/BaseButton.vue";

defineProps<{
  visible: boolean;
  adjusting: boolean;
  error: string;
}>();

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();
</script>

<template>
  <Teleport to="body">
    <Transition name="hot-debug-modal">
      <div
        v-if="visible"
        class="hot-debug-overlay"
        @mousedown.self="emit('cancel')"
      >
        <div
          class="hot-debug-dialog"
          role="dialog"
          aria-modal="true"
          :aria-label="t('hotReload.debugPrompt.title')"
        >
          <div class="hot-debug-head">
            <span class="hot-debug-title">{{ t("hotReload.debugPrompt.title") }}</span>
          </div>
          <div class="hot-debug-body">
            <p class="hot-debug-text">{{ t("hotReload.debugPrompt.message") }}</p>
            <p class="hot-debug-note">{{ t("hotReload.debugPrompt.note") }}</p>
            <p v-if="error" class="hot-debug-error">{{ error }}</p>
          </div>
          <div class="hot-debug-footer">
            <BaseButton variant="primary" type="button" :disabled="adjusting" @click="emit('confirm')">
              {{ adjusting ? t("hotReload.debugPrompt.adjusting") : t("hotReload.debugPrompt.confirm") }}
            </BaseButton>
            <BaseButton type="button" :disabled="adjusting" @click="emit('cancel')">
              {{ t("hotReload.debugPrompt.cancel") }}
            </BaseButton>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.hot-debug-overlay {
  position: fixed;
  inset: 0;
  background: rgba(8, 10, 14, 0.42);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.hot-debug-dialog {
  width: 440px;
  max-width: calc(100% - 48px);
  background: var(--surface-elevated, var(--panel-bg));
  border: 1px solid var(--border-color);
  border-radius: 12px;
  box-shadow: 0 18px 40px rgba(15, 17, 21, 0.24);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.hot-debug-head {
  padding: 16px 20px 10px;
  border-bottom: 1px solid var(--border-color);
}

.hot-debug-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-color);
}

.hot-debug-body {
  padding: 14px 20px 4px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.hot-debug-text {
  margin: 0;
  font-size: 12.5px;
  line-height: 1.55;
  color: var(--text-color);
}

.hot-debug-note {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.hot-debug-error {
  margin: 4px 0 0;
  padding: 8px 10px;
  border-radius: 6px;
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 11.5px;
  line-height: 1.5;
  word-break: break-word;
}

.hot-debug-footer {
  display: flex;
  gap: 8px;
  padding: 14px 20px 16px;
}

.hot-debug-modal-enter-active,
.hot-debug-modal-leave-active {
  transition: opacity 0.15s ease;
}

.hot-debug-modal-enter-active .hot-debug-dialog,
.hot-debug-modal-leave-active .hot-debug-dialog {
  transition: transform 0.15s ease;
}

.hot-debug-modal-enter-from,
.hot-debug-modal-leave-to {
  opacity: 0;
}

.hot-debug-modal-enter-from .hot-debug-dialog,
.hot-debug-modal-leave-to .hot-debug-dialog {
  transform: scale(0.95) translateY(8px);
}
</style>
