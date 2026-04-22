<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { SegmentedOption } from "../ui/BaseSegmented.vue";

const props = defineProps<{
  query: string;
  searching: boolean;
  scope?: string;
  scopeOptions?: SegmentedOption[];
}>();

const emit = defineEmits<{
  (e: "update:query", value: string): void;
  (e: "update:scope", value: string): void;
  (e: "clear"): void;
}>();

function onInput(e: Event) {
  const target = e.target as HTMLInputElement;
  emit("update:query", target.value);
}

const hasScopeToggle = computed(() =>
  !!props.scope && (props.scopeOptions?.length ?? 0) > 1,
);

const currentScopeOption = computed<SegmentedOption | null>(() => {
  if (!props.scope || !props.scopeOptions?.length) return null;
  return props.scopeOptions.find((option) => option.value === props.scope)
    ?? props.scopeOptions[0]
    ?? null;
});

const nextScopeOption = computed<SegmentedOption | null>(() => {
  if (!props.scopeOptions?.length) return null;
  const currentIndex = props.scopeOptions.findIndex((option) => option.value === props.scope);
  if (currentIndex < 0) return props.scopeOptions[0] ?? null;
  return props.scopeOptions[(currentIndex + 1) % props.scopeOptions.length] ?? null;
});

const scopeTitle = computed(() => currentScopeOption.value?.label ?? "");
const searchPlaceholder = computed(() => {
  if (props.scope === "folder") return t("asset.search.placeholder.folder");
  if (props.scope === "global") return t("asset.search.placeholder.global");
  return t("asset.search.placeholder");
});

function toggleScope() {
  const next = nextScopeOption.value;
  if (!next || next.value === props.scope) return;
  emit("update:scope", next.value);
}
</script>

<template>
  <div class="asb-wrap">
    <div class="asb-main">
      <div class="asb-bar">
        <span class="asb-icon" :class="{ spinning: props.searching }">
          <svg
            v-if="!props.searching"
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
          type="text"
          class="asb-input"
          :value="props.query"
          :placeholder="searchPlaceholder"
          @input="onInput"
        />
        <button
          v-if="props.query"
          class="asb-clear"
          type="button"
          @click="emit('clear')"
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
      </div>

      <button
        v-if="hasScopeToggle"
        type="button"
        class="asb-scope-button"
        :title="scopeTitle"
        :aria-label="scopeTitle"
        @click="toggleScope"
      >
        <span class="asb-scope-icon" :class="props.scope" aria-hidden="true">
          <svg
            v-if="props.scope === 'folder'"
            viewBox="0 0 16 16"
            width="12"
            height="12"
            fill="none"
          >
            <path
              d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
              fill="currentColor"
            />
          </svg>
          <svg
            v-else
            viewBox="0 0 16 16"
            width="12"
            height="12"
            fill="none"
          >
            <circle cx="8" cy="8" r="5.25" stroke="currentColor" stroke-width="1.2" />
            <path
              d="M8 2.75c1.45 1.42 2.25 3.27 2.25 5.25S9.45 11.83 8 13.25"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linecap="round"
            />
            <path
              d="M8 2.75C6.55 4.17 5.75 6.02 5.75 8S6.55 11.83 8 13.25"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linecap="round"
            />
            <path
              d="M2.75 8h10.5"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linecap="round"
            />
          </svg>
        </span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.asb-wrap {
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  flex-shrink: 0;
}

.asb-main {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.asb-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 36px;
  padding: 0 10px 0 12px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg, var(--hover-bg)) 24%);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  transition: border-color 0.15s, box-shadow 0.15s;
  min-width: 0;
  flex: 1;
}

.asb-bar:hover {
  border-color: color-mix(in srgb, var(--text-secondary) 50%, var(--border-color));
}

.asb-bar:focus-within {
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 12%, transparent);
}

/* ── Left icon ───────────────────────────────── */
.asb-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  color: var(--text-secondary);
  flex-shrink: 0;
}

.asb-icon.spinning svg {
  animation: asb-spin 0.9s linear infinite;
  transform-origin: center;
}

@keyframes asb-spin {
  to {
    transform: rotate(360deg);
  }
}

/* ── Input ───────────────────────────────────── */
.asb-input {
  flex: 1;
  min-width: 0;
  height: 100%;
  background: transparent;
  border: none;
  outline: none;
  font-size: 13px;
  color: var(--text-color);
  font-family: inherit;
  padding: 0;
}

.asb-input::placeholder {
  color: var(--text-secondary);
  opacity: 0.7;
}

/* ── Right side ──────────────────────────────── */
.asb-clear {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  background: none;
  border: none;
  border-radius: 4px;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

.asb-clear:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.asb-scope-button {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 36px;
  width: 36px;
  min-width: 36px;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.asb-scope-button:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.asb-scope-button:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -1px;
}

.asb-scope-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  flex-shrink: 0;
}

.asb-scope-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
}

.asb-scope-icon.global {
  color: color-mix(in srgb, var(--accent-color) 18%, var(--text-secondary) 82%);
}

</style>
