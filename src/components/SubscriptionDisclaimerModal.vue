<script setup lang="ts">
import { t } from "../i18n";
import BaseButton from "./ui/BaseButton.vue";

defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  cancel: [];
}>();
</script>

<template>
  <Teleport to="body">
    <Transition name="subscription-disclaimer">
      <div
        v-if="open"
        class="subscription-disclaimer-overlay"
        @mousedown.self="emit('cancel')"
      >
        <section
          class="subscription-disclaimer-modal"
          role="dialog"
          aria-modal="true"
          :aria-label="t('disclaimer.subscription.title')"
        >
          <header class="subscription-disclaimer-header">
            <div class="subscription-disclaimer-header-copy">
              <span class="subscription-disclaimer-title">
                {{ t("disclaimer.subscription.title") }}
              </span>
              <p class="subscription-disclaimer-text">
                {{ t("disclaimer.subscription.body") }}
              </p>
            </div>
            <button
              class="subscription-disclaimer-close"
              type="button"
              :aria-label="t('common.close')"
              @click="emit('cancel')"
            >
              <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
                <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
              </svg>
            </button>
          </header>

          <div class="subscription-disclaimer-body">
            <div class="subscription-disclaimer-note">
              <ul class="subscription-disclaimer-list">
                <li>{{ t("disclaimer.subscription.risk1") }}</li>
                <li>{{ t("disclaimer.subscription.risk2") }}</li>
                <li>{{ t("disclaimer.subscription.risk3") }}</li>
              </ul>
            </div>
          </div>

          <footer class="subscription-disclaimer-footer">
            <BaseButton size="md" @click="emit('cancel')">
              {{ t("disclaimer.subscription.cancel") }}
            </BaseButton>
          </footer>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.subscription-disclaimer-overlay {
  position: fixed;
  inset: 0;
  z-index: 10001;
  background: rgba(8, 10, 14, 0.34);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.subscription-disclaimer-modal {
  width: min(520px, 100%);
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 12px;
  background: var(--surface-elevated);
  box-shadow: 0 18px 40px rgba(15, 17, 21, 0.16);
  overflow: hidden;
}

.subscription-disclaimer-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 18px 20px 14px;
  border-bottom: 1px solid var(--border-color);
}

.subscription-disclaimer-header-copy {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.subscription-disclaimer-title {
  font-size: 15px;
  font-weight: 700;
  line-height: 1.35;
  color: var(--text-color);
}

.subscription-disclaimer-close {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.subscription-disclaimer-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.subscription-disclaimer-body {
  padding: 16px 20px;
}

.subscription-disclaimer-text {
  margin: 0;
  font-size: 13px;
  line-height: 1.65;
  color: var(--text-secondary);
}

.subscription-disclaimer-note {
  border: 1px solid color-mix(in srgb, var(--status-danger-border) 72%, var(--border-color) 28%);
  border-radius: 10px;
  background: color-mix(in srgb, var(--status-danger-bg) 76%, var(--surface-elevated) 24%);
  padding: 12px 14px 12px 16px;
}

.subscription-disclaimer-list {
  margin: 0;
  padding-left: 18px;
  font-size: 13px;
  line-height: 1.65;
  color: var(--text-color);
}

.subscription-disclaimer-list li + li {
  margin-top: 8px;
}

.subscription-disclaimer-list li::marker {
  color: var(--status-danger-fg);
}

.subscription-disclaimer-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 14px 20px 18px;
  border-top: 1px solid var(--border-color);
}

.subscription-disclaimer-enter-active,
.subscription-disclaimer-leave-active {
  transition: opacity 0.15s ease;
}

.subscription-disclaimer-enter-active .subscription-disclaimer-modal,
.subscription-disclaimer-leave-active .subscription-disclaimer-modal {
  transition: transform 0.15s ease, opacity 0.15s ease;
}

.subscription-disclaimer-enter-from,
.subscription-disclaimer-leave-to {
  opacity: 0;
}

.subscription-disclaimer-enter-from .subscription-disclaimer-modal,
.subscription-disclaimer-leave-to .subscription-disclaimer-modal {
  opacity: 0;
  transform: scale(0.96) translateY(8px);
}
</style>
