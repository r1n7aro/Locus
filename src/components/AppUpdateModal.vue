<script setup lang="ts">
import { computed } from "vue";
import type { AppUpdateInfo } from "../types";
import { t } from "../i18n";
import BaseButton from "./ui/BaseButton.vue";

const props = defineProps<{
  open: boolean;
  info: AppUpdateInfo | null;
}>();

const emit = defineEmits<{
  close: [];
  view: [];
}>();

const visibleChanges = computed(() => props.info?.changes ?? []);
const latestVersionLabel = computed(() => {
  if (!props.info) return "";
  return `Locus v${props.info.latestVersion} (${props.info.releasedAt})`;
});

const currentVersionLabel = computed(() => {
  if (!props.info) return "";
  return `Locus v${props.info.currentVersion}`;
});
</script>

<template>
  <Teleport to="body">
    <Transition name="app-update-modal">
      <div
        v-if="open && info"
        class="app-update-overlay"
        @mousedown.self="emit('close')"
      >
        <section
          class="app-update-dialog"
          role="dialog"
          aria-modal="true"
          :aria-label="info.title"
        >
          <header class="app-update-header">
            <div class="app-update-header-copy">
              <span class="app-update-title">{{ info.title }}</span>
            </div>
            <button
              class="app-update-close"
              type="button"
              :aria-label="t('common.close')"
              @click="emit('close')"
            >
              <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
                <path d="M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.75.75 0 1 1 1.06 1.06L9.06 8l3.22 3.22a.75.75 0 1 1-1.06 1.06L8 9.06l-3.22 3.22a.75.75 0 0 1-1.06-1.06L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06z"/>
              </svg>
            </button>
          </header>

          <div class="app-update-body">
            <div class="app-update-version-row">
              <div class="app-update-version-block">
                <div class="app-update-version-label">
                  {{ t("app.update.latestVersion") }}
                </div>
                <div class="app-update-version-text app-update-version-text-latest">
                  {{ latestVersionLabel }}
                </div>
              </div>

              <div class="app-update-version-block app-update-version-block-current">
                <div class="app-update-version-label">
                  {{ t("app.update.currentVersion") }}
                </div>
                <div class="app-update-version-text app-update-version-text-current">
                  {{ currentVersionLabel }}
                </div>
              </div>
            </div>

            <section v-if="visibleChanges.length > 0" class="app-update-changes">
              <div
                v-for="group in visibleChanges"
                :key="group.title"
                class="app-update-group"
              >
                <div class="app-update-group-title">{{ group.title }}</div>
                <ul class="app-update-group-list">
                  <li
                    v-for="item in group.items"
                    :key="item"
                  >
                    {{ item }}
                  </li>
                </ul>
              </div>
            </section>
          </div>

          <footer class="app-update-footer">
            <BaseButton size="md" @click="emit('close')">
              {{ t("app.update.dismiss") }}
            </BaseButton>
            <BaseButton variant="primary" size="md" @click="emit('view')">
              {{ t("app.update.updateVersion") }}
            </BaseButton>
          </footer>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.app-update-overlay {
  position: fixed;
  inset: 0;
  z-index: 10001;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: rgba(8, 10, 14, 0.34);
}

.app-update-dialog {
  width: min(620px, 100%);
  max-height: min(680px, calc(100vh - 40px));
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 12px;
  background: var(--surface-elevated);
  box-shadow: 0 18px 40px rgba(15, 17, 21, 0.16);
  overflow: hidden;
}

.app-update-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 18px 20px 14px;
}

.app-update-header-copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.app-update-title {
  font-size: 18px;
  font-weight: 700;
  line-height: 1.3;
  color: var(--text-color);
}

.app-update-close {
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

.app-update-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.app-update-body {
  display: flex;
  flex-direction: column;
  gap: 0;
  padding: 0 20px 0;
  overflow: auto;
}

.app-update-version-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 0.92fr);
  gap: 14px;
  align-items: center;
  padding: 0 0 14px;
  border-bottom: 1px solid var(--border-color);
}

.app-update-version-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.app-update-version-block-current {
  min-width: 0;
  padding-left: 12px;
  border-left: 1px solid var(--border-color);
  justify-self: start;
}

.app-update-version-label {
  font-size: 12px;
  font-weight: 600;
  line-height: 1.4;
  color: var(--text-secondary);
}

.app-update-version-text {
  font-size: 13px;
  line-height: 1.5;
  font-weight: 700;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
  word-break: break-word;
}

.app-update-version-text-latest {
  font-size: 13px;
  line-height: 1.5;
  font-weight: 700;
}

.app-update-version-text-current {
  font-size: 13px;
  line-height: 1.5;
  font-weight: 700;
}

.app-update-changes {
  display: flex;
  flex-direction: column;
  gap: 28px;
  padding: 14px 0 18px;
}

.app-update-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.app-update-group-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.app-update-group-list {
  margin: 0;
  padding-left: 18px;
  font-size: 13px;
  line-height: 1.65;
  color: var(--text-secondary);
}

.app-update-group-list li + li {
  margin-top: 6px;
}

.app-update-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 14px 20px 18px;
  border-top: 1px solid var(--border-color);
}

.app-update-modal-enter-active,
.app-update-modal-leave-active {
  transition: opacity 0.15s ease;
}

.app-update-modal-enter-active .app-update-dialog,
.app-update-modal-leave-active .app-update-dialog {
  transition: transform 0.15s ease, opacity 0.15s ease;
}

.app-update-modal-enter-from,
.app-update-modal-leave-to {
  opacity: 0;
}

.app-update-modal-enter-from .app-update-dialog,
.app-update-modal-leave-to .app-update-dialog {
  opacity: 0;
  transform: scale(0.96) translateY(8px);
}

@media (max-width: 720px) {
  .app-update-dialog {
    max-height: min(720px, calc(100vh - 24px));
  }

  .app-update-version-row {
    grid-template-columns: 1fr;
    gap: 12px;
  }

  .app-update-version-block-current {
    min-width: 0;
    padding-top: 10px;
    padding-left: 0;
    border-top: 1px solid var(--border-color);
    border-left: none;
  }

  .app-update-overlay {
    padding: 12px;
  }
}
</style>
