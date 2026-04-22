<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { t } from "../../i18n";

defineProps<{
  query: string;
  searching: boolean;
}>();

const emit = defineEmits<{
  (e: "update:query", value: string): void;
  (e: "clear"): void;
}>();

const inputRef = ref<HTMLInputElement | null>(null);

const isMac = computed(() => {
  if (typeof navigator === "undefined") return false;
  return /Mac|iPhone|iPod|iPad/i.test(navigator.platform);
});

const shortcutLabel = computed(() => (isMac.value ? "⌘ F" : "Ctrl F"));

function onGlobalKeydown(e: KeyboardEvent) {
  // Ctrl+F (or Cmd+F on macOS) → focus the search input
  if ((e.ctrlKey || e.metaKey) && (e.key === "f" || e.key === "F")) {
    e.preventDefault();
    inputRef.value?.focus();
    inputRef.value?.select();
  }
}

onMounted(() => {
  window.addEventListener("keydown", onGlobalKeydown);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onGlobalKeydown);
});
</script>

<template>
  <div class="kx-search-wrap">
    <div class="kx-search-bar">
      <!-- Left icon: search glyph or spinner while loading -->
      <span class="kx-search-icon" :class="{ spinning: searching }">
        <svg
          v-if="!searching"
          viewBox="0 0 16 16"
          width="16"
          height="16"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="7" cy="7" r="4.5" />
          <line x1="10.5" y1="10.5" x2="14" y2="14" />
        </svg>
        <svg
          v-else
          viewBox="0 0 16 16"
          width="16"
          height="16"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
        >
          <path d="M8 1.5 a 6.5 6.5 0 1 0 6.5 6.5" />
        </svg>
      </span>

      <input
        ref="inputRef"
        class="kx-search-input"
        type="text"
        :value="query"
        :placeholder="t('knowledge.search.globalPlaceholder')"
        @input="emit('update:query', ($event.target as HTMLInputElement).value)"
      />

      <!-- Right side: clear button when typing, otherwise shortcut hint -->
      <button
        v-if="query"
        class="kx-clear-btn"
        type="button"
        @click="emit('clear')"
        :title="t('common.close')"
      >
        <svg
          viewBox="0 0 16 16"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
        >
          <line x1="4" y1="4" x2="12" y2="12" />
          <line x1="12" y1="4" x2="4" y2="12" />
        </svg>
      </button>
      <span v-else class="kx-shortcut-hint">{{ shortcutLabel }}</span>
    </div>
  </div>
</template>

<style scoped>
.kx-search-wrap {
  padding: 8px 14px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color));
  flex-shrink: 0;
}

.kx-search-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 9px 0 10px;
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--input-bg, var(--hover-bg)) 26%);
  border: 1px solid var(--border-color);
  border-radius: 7px;
  transition: border-color 0.15s, box-shadow 0.15s;
}

.kx-search-bar:hover {
  border-color: color-mix(in srgb, var(--text-secondary) 50%, var(--border-color));
}

.kx-search-bar:focus-within {
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 14%, transparent);
}

/* ── Left icon ───────────────────────────────── */
.kx-search-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  color: var(--text-secondary);
  flex-shrink: 0;
}

.kx-search-icon.spinning svg {
  animation: kx-spin 0.9s linear infinite;
  transform-origin: center;
}

@keyframes kx-spin {
  to {
    transform: rotate(360deg);
  }
}

/* ── Input ───────────────────────────────────── */
.kx-search-input {
  flex: 1;
  min-width: 0;
  height: 100%;
  background: transparent;
  border: none;
  outline: none;
  font-size: 12px;
  color: var(--text-color);
  font-family: inherit;
  padding: 0;
}

.kx-search-input::placeholder {
  color: var(--text-secondary);
  opacity: 0.7;
}

/* ── Right side ──────────────────────────────── */
.kx-shortcut-hint {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  height: 18px;
  padding: 0 6px;
  font-size: 10px;
  font-weight: 500;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color));
  border: 1px solid var(--border-color);
  border-radius: 4px;
  letter-spacing: 0.02em;
}

.kx-clear-btn {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  background: none;
  border: none;
  border-radius: 4px;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

.kx-clear-btn:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}
</style>
