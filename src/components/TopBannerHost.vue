<script setup lang="ts">
import { useNotificationStore } from "../stores/notification";

const notificationStore = useNotificationStore();
</script>

<template>
  <TransitionGroup name="banner-slide" tag="div" class="top-banner-host">
    <div
      v-for="notice in notificationStore.visibleNotices"
      :key="notice.id"
      class="banner-notice"
      :class="'banner-' + notice.level"
      @mouseenter="notificationStore.pauseNotice(notice.id)"
      @mouseleave="notificationStore.resumeNotice(notice.id)"
    >
      <span v-if="notice.spinner" class="banner-spinner" aria-hidden="true"></span>
      <svg v-else-if="notice.level === 'error'" class="banner-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
        <path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1zm-.75 4a.75.75 0 0 1 1.5 0v3a.75.75 0 0 1-1.5 0V5zm.75 6.5a.75.75 0 1 1 0-1.5.75.75 0 0 1 0 1.5z"/>
      </svg>
      <svg v-else-if="notice.level === 'warning'" class="banner-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
        <path d="M8.22 1.754a.25.25 0 0 0-.44 0L1.06 13.484a.25.25 0 0 0 .22.37h13.44a.25.25 0 0 0 .22-.37L8.22 1.754zM7.25 10V7a.75.75 0 0 1 1.5 0v3a.75.75 0 0 1-1.5 0zm.75 3a.75.75 0 1 1 0-1.5.75.75 0 0 1 0 1.5z"/>
      </svg>
      <svg v-else-if="notice.level === 'success'" class="banner-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
        <path d="M8 16A8 8 0 1 1 8 0a8 8 0 0 1 0 16zm3.78-9.72a.75.75 0 0 0-1.06-1.06L7 8.94 5.28 7.22a.75.75 0 0 0-1.06 1.06l2.25 2.25a.75.75 0 0 0 1.06 0l4.25-4.25z"/>
      </svg>
      <svg v-else class="banner-icon" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
        <path d="M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zm0 9.5a.75.75 0 1 1 0-1.5.75.75 0 0 1 0 1.5zm.75-3a.75.75 0 0 1-1.5 0V5a.75.75 0 0 1 1.5 0v3z"/>
      </svg>
      <span class="banner-msg">{{ notice.message }}</span>
      <button class="banner-close" @click="notificationStore.removeNotice(notice.id)">
        <svg viewBox="0 0 12 12" width="10" height="10">
          <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
        </svg>
      </button>
    </div>
  </TransitionGroup>
</template>

<style scoped>
.top-banner-host {
  pointer-events: none;
  position: fixed;
  top: 48px;
  left: 16px;
  right: 16px;
  z-index: 180;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}

.banner-notice {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: 7px;
  width: min(760px, 100%);
  padding: 6px 10px;
  font-size: 12px;
  line-height: 1.35;
  border-radius: 6px;
  border: 1px solid color-mix(in srgb, currentColor 12%, transparent);
  border-left-width: 2px;
  box-shadow: 0 4px 12px rgba(15, 23, 42, 0.08);
  backdrop-filter: blur(8px);
  user-select: text;
}

.banner-error {
  background: color-mix(in srgb, var(--status-danger-bg) 78%, var(--panel-bg) 22%);
  border-left-color: var(--status-danger-fg);
  color: var(--status-danger-fg);
}
.banner-warning {
  background: color-mix(in srgb, var(--status-warn-bg) 78%, var(--panel-bg) 22%);
  border-left-color: var(--status-warn-fg);
  color: var(--status-warn-fg);
}
.banner-success {
  background: color-mix(in srgb, var(--status-good-bg) 78%, var(--panel-bg) 22%);
  border-left-color: var(--status-good-fg);
  color: var(--status-good-fg);
}
.banner-info {
  background: color-mix(in srgb, var(--accent-soft) 78%, var(--panel-bg) 22%);
  border-left-color: var(--accent-color);
  color: color-mix(in srgb, var(--accent-color) 78%, var(--text-color) 22%);
}

.banner-icon {
  flex-shrink: 0;
  opacity: 0.8;
}

.banner-spinner {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
  border-radius: 999px;
  border: 2px solid color-mix(in srgb, currentColor 18%, transparent);
  border-top-color: currentColor;
  animation: banner-spin 0.8s linear infinite;
}
.banner-msg {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.banner-close {
  flex-shrink: 0;
  background: none;
  border: none;
  cursor: pointer;
  padding: 1px;
  border-radius: 2px;
  color: inherit;
  opacity: 0.6;
  display: flex;
  align-items: center;
}
.banner-close:hover {
  opacity: 1;
  background: rgba(0, 0, 0, 0.08);
}

/* Slide transition */
.banner-slide-enter-active,
.banner-slide-leave-active {
  transition: max-height 0.25s ease, opacity 0.25s ease, transform 0.25s ease;
  overflow: hidden;
}
.banner-slide-enter-from,
.banner-slide-leave-to {
  max-height: 0;
  opacity: 0;
  transform: translateY(-8px);
}
.banner-slide-enter-to,
.banner-slide-leave-from {
  max-height: 56px;
  opacity: 1;
  transform: translateY(0);
}

@keyframes banner-spin {
  to {
    transform: rotate(360deg);
  }
}

@media (max-width: 720px) {
  .top-banner-host {
    top: 44px;
    left: 10px;
    right: 10px;
  }

  .banner-notice {
    width: 100%;
    padding: 6px 8px;
  }
}
</style>
