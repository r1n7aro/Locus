<script setup lang="ts">
import { computed, useSlots } from "vue";

const slots = useSlots();

const hasTopBar = computed(() => !!slots["top-start"] || !!slots["top-end"]);
const hasFooter = computed(() => !!slots.footer);
</script>

<template>
  <div class="chat-input-shell">
    <div v-if="hasTopBar" class="chat-input-shell-topbar">
      <div class="chat-input-shell-topbar-start">
        <slot name="top-start" />
      </div>
      <div class="chat-input-shell-topbar-end">
        <slot name="top-end" />
      </div>
    </div>

    <div class="chat-input-shell-body">
      <slot name="floating" />
      <div class="chat-input-shell-stack">
        <slot name="before-composer" />
        <slot />
      </div>
    </div>

    <div v-if="hasFooter" class="chat-input-shell-footer">
      <slot name="footer" />
    </div>
  </div>
</template>

<style scoped>
.chat-input-shell {
  display: flex;
  flex-direction: column;
  width: 100%;
  min-width: 0;
}

.chat-input-shell-topbar {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
  min-height: 28px;
}

.chat-input-shell-topbar-start {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
}

.chat-input-shell-topbar-end {
  flex-shrink: 0;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
}

.chat-input-shell-body {
  position: relative;
  width: 100%;
  min-width: 0;
}

.chat-input-shell-stack {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
  min-width: 0;
}

.chat-input-shell-footer {
  display: flex;
  align-items: center;
  margin-top: 8px;
}
</style>
