<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { searchWorkspaceAssets } from "../../services/asset";
import {
  filterUnityObjectReferenceSearchResults,
  unityObjectReferenceAssetKey,
  unityObjectReferenceDisplayParts,
  unityObjectReferenceSearchQuery,
  unityObjectReferenceTypeHint,
  unityObjectReferenceValueForSearchResult,
  UNITY_OBJECT_REFERENCE_SEARCH_ROOTS,
  type UnityObjectReferenceFilter,
} from "../../services/unityObjectReferencePicker";
import type { AssetSearchResult } from "../../types";
import { unitySerializedValueToEditText } from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  displayValue?: string;
  disabled?: boolean;
  readonly?: boolean;
  placeholder?: string;
  title?: string;
  ariaLabel?: string;
  referenceTypeFullName?: string;
  referenceTypeAssembly?: string;
  searchRoots?: string[];
  searchLimit?: number;
}>(), {
  displayValue: "",
  disabled: false,
  readonly: false,
  placeholder: "Assets/...",
  title: "",
  ariaLabel: "",
  referenceTypeFullName: "",
  referenceTypeAssembly: "",
  searchRoots: () => [...UNITY_OBJECT_REFERENCE_SEARCH_ROOTS],
  searchLimit: 24,
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  edit: [value: string];
  commit: [value: string];
}>();

const displayText = ref(unitySerializedValueToEditText("ObjectReference", props.modelValue, props.displayValue));
const searchText = ref("");
const open = ref(false);
const focused = ref(false);
const searching = ref(false);
const searchError = ref("");
const results = ref<AssetSearchResult[]>([]);
const highlightedIndex = ref(-1);
const rootEl = ref<HTMLElement | null>(null);
const searchInputEl = ref<HTMLInputElement | null>(null);
let debounceTimer: number | null = null;
let blurTimer: number | null = null;
let searchRun = 0;

const editable = computed(() => !props.disabled && !props.readonly);
const typeHint = computed(() => unityObjectReferenceTypeHint(props.referenceTypeFullName));
const displayParts = computed(() => unityObjectReferenceDisplayParts(displayText.value));
const filter = computed<UnityObjectReferenceFilter>(() => ({
  referenceTypeFullName: props.referenceTypeFullName,
  referenceTypeAssembly: props.referenceTypeAssembly,
  currentValue: displayText.value,
  limit: props.searchLimit,
}));
const searchRoots = computed(() =>
  props.searchRoots.length ? props.searchRoots : [...UNITY_OBJECT_REFERENCE_SEARCH_ROOTS],
);
const dropdownVisible = computed(() => open.value && editable.value);
const showEmpty = computed(() =>
  dropdownVisible.value && !searching.value && !searchError.value && results.value.length === 0,
);

watch(
  () => [props.modelValue, props.displayValue] as const,
  () => {
    displayText.value = unitySerializedValueToEditText("ObjectReference", props.modelValue, props.displayValue);
  },
);

watch(
  () => [props.referenceTypeFullName, props.referenceTypeAssembly] as const,
  () => {
    if (open.value) scheduleSearch(true);
  },
);

onBeforeUnmount(() => {
  clearDebounce();
  clearBlurTimer();
});

function clearDebounce() {
  if (debounceTimer === null) return;
  window.clearTimeout(debounceTimer);
  debounceTimer = null;
}

function clearBlurTimer() {
  if (blurTimer === null) return;
  window.clearTimeout(blurTimer);
  blurTimer = null;
}

function updateSearchText(event: Event) {
  const target = event.target as HTMLInputElement | null;
  searchText.value = target?.value ?? "";
  if (editable.value) {
    open.value = true;
    scheduleSearch(false);
  }
}

function beginEdit() {
  if (!editable.value) return;
  const shouldResetSearch = !open.value;
  focused.value = true;
  clearBlurTimer();
  if (shouldResetSearch) searchText.value = "";
  open.value = true;
  scheduleSearch(true);
  focusSearchInput();
}

function handleDisplayFocus() {
  if (!editable.value) return;
  focused.value = true;
  clearBlurTimer();
}

function toggleEdit() {
  if (!editable.value) return;
  if (open.value) {
    closeDropdown();
    return;
  }
  beginEdit();
}

function focusSearchInput() {
  void nextTick(() => {
    if (!open.value || !editable.value) return;
    searchInputEl.value?.focus();
    searchInputEl.value?.select();
  });
}

function scheduleBlurCheck() {
  clearBlurTimer();
  blurTimer = window.setTimeout(() => {
    const active = document.activeElement;
    if (active && rootEl.value?.contains(active)) return;
    focused.value = false;
    closeDropdown();
  }, 80);
}

function closeDropdown() {
  searchRun += 1;
  open.value = false;
  searchText.value = "";
  searching.value = false;
  searchError.value = "";
  results.value = [];
  highlightedIndex.value = -1;
  clearDebounce();
}

function scheduleSearch(immediate: boolean) {
  clearDebounce();
  if (!editable.value) return;
  const delay = immediate ? 0 : 160;
  debounceTimer = window.setTimeout(() => {
    debounceTimer = null;
    void runSearch();
  }, delay);
}

async function runSearch() {
  const run = ++searchRun;
  const query = unityObjectReferenceSearchQuery(searchText.value, filter.value);
  if (!query) {
    results.value = [];
    searching.value = false;
    searchError.value = "";
    highlightedIndex.value = -1;
    return;
  }
  searching.value = true;
  searchError.value = "";
  try {
    const rawResults = await searchWorkspaceAssets(query, searchRoots.value, props.searchLimit * 3);
    if (run !== searchRun) return;
    results.value = filterUnityObjectReferenceSearchResults(rawResults, filter.value);
    highlightedIndex.value = results.value.length > 0 ? 0 : -1;
  } catch (error) {
    if (run !== searchRun) return;
    results.value = [];
    highlightedIndex.value = -1;
    searchError.value = error instanceof Error ? error.message : String(error);
  } finally {
    if (run === searchRun) searching.value = false;
  }
}

function selectResult(result: AssetSearchResult) {
  if (!editable.value) return;
  const value = unityObjectReferenceValueForSearchResult(result);
  displayText.value = value;
  emit("edit", value);
  emit("update:modelValue", value);
  emit("commit", value);
  closeDropdown();
}

function clearReference() {
  if (!editable.value) return;
  displayText.value = "";
  emit("edit", "");
  emit("update:modelValue", "");
  emit("commit", "");
  closeDropdown();
}

function moveHighlight(delta: number) {
  if (!open.value) {
    beginEdit();
    return;
  }
  if (!results.value.length) return;
  const next = highlightedIndex.value < 0
    ? 0
    : (highlightedIndex.value + delta + results.value.length) % results.value.length;
  highlightedIndex.value = next;
}

function handleDisplayKeydown(event: KeyboardEvent) {
  if (!editable.value) return;
  if (event.key === "Enter" || event.key === " " || event.key === "ArrowDown") {
    beginEdit();
    event.preventDefault();
    return;
  }
  if (event.key === "Escape") {
    closeDropdown();
    event.preventDefault();
  }
}

function handleSearchKeydown(event: KeyboardEvent) {
  if (!editable.value) return;
  if (event.key === "ArrowDown") {
    moveHighlight(1);
    event.preventDefault();
    return;
  }
  if (event.key === "ArrowUp") {
    moveHighlight(-1);
    event.preventDefault();
    return;
  }
  if (event.key === "Escape") {
    closeDropdown();
    event.preventDefault();
    return;
  }
  if (event.key === "Enter") {
    if (open.value && highlightedIndex.value >= 0 && results.value[highlightedIndex.value]) {
      selectResult(results.value[highlightedIndex.value]);
      event.preventDefault();
      return;
    }
    event.preventDefault();
  }
}
</script>

<template>
  <div
    ref="rootEl"
    class="unity-object-reference-field"
    :class="{ focused, disabled: disabled || readonly }"
  >
    <button
      class="unity-object-reference-display"
      type="button"
      :disabled="disabled"
      :aria-disabled="readonly ? 'true' : undefined"
      :title="displayText || title || undefined"
      :aria-label="ariaLabel || undefined"
      @focus="handleDisplayFocus"
      @click="toggleEdit"
      @blur="scheduleBlurCheck"
      @keydown="handleDisplayKeydown"
    >
      <span v-if="displayText" class="unity-object-reference-display-content">
        <span class="unity-object-reference-display-name">{{ displayParts.name }}</span>
      </span>
      <span v-else class="unity-object-reference-placeholder">{{ placeholder }}</span>
    </button>
    <button
      v-if="displayText && editable"
      class="unity-object-reference-clear"
      type="button"
      title="None"
      aria-label="None"
      @mousedown.prevent
      @click="clearReference"
    >
      x
    </button>
    <div
      v-if="dropdownVisible"
      class="unity-object-reference-dropdown"
      role="dialog"
      :aria-label="`${typeHint} assets`"
    >
      <div class="unity-object-reference-search">
        <input
          ref="searchInputEl"
          class="unity-object-reference-search-input"
          type="text"
          :value="searchText"
          :placeholder="`Search ${typeHint}`"
          :aria-label="`Search ${typeHint}`"
          autocomplete="off"
          @input="updateSearchText"
          @keydown="handleSearchKeydown"
          @blur="scheduleBlurCheck"
        />
      </div>
      <button
        type="button"
        class="unity-object-reference-option unity-object-reference-none"
        @mousedown.prevent
        @click="clearReference"
      >
        <span class="unity-object-reference-option-name">None</span>
        <span class="unity-object-reference-option-path">{{ typeHint }}</span>
      </button>
      <div v-if="searching" class="unity-object-reference-state">Loading...</div>
      <div v-else-if="searchError" class="unity-object-reference-state">{{ searchError }}</div>
      <div v-else-if="showEmpty" class="unity-object-reference-state">No matches</div>
      <button
        v-for="(result, index) in results"
        :key="unityObjectReferenceAssetKey(result)"
        type="button"
        class="unity-object-reference-option"
        :class="{ highlighted: highlightedIndex === index }"
        :title="unityObjectReferenceValueForSearchResult(result)"
        :aria-selected="highlightedIndex === index"
        @mousedown.prevent
        @mouseenter="highlightedIndex = index"
        @click="selectResult(result)"
      >
        <span class="unity-object-reference-option-main">
          <span class="unity-object-reference-option-name">{{ result.name }}</span>
          <span class="unity-object-reference-option-kind">{{ result.typeLabel || result.kind }}</span>
        </span>
        <span class="unity-object-reference-option-path">
          {{ unityObjectReferenceValueForSearchResult(result) }}
        </span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.unity-object-reference-field {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  position: relative;
  display: flex;
  align-items: center;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-mono-identifier);
  box-sizing: border-box;
}

.unity-object-reference-field.focused {
  border-color: var(--accent-color);
}

.unity-object-reference-field.disabled {
  opacity: 0.65;
}

.unity-object-reference-display {
  flex: 1 1 auto;
  width: 100%;
  min-width: 0;
  min-height: 24px;
  padding: 0 7px;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  font-family: inherit;
  box-sizing: border-box;
  cursor: pointer;
  text-align: left;
}

.unity-object-reference-display:focus {
  outline: none;
}

.unity-object-reference-display-content {
  width: 100%;
  min-width: 0;
  display: block;
}

.unity-object-reference-display-name,
.unity-object-reference-placeholder {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-object-reference-display-name {
  display: block;
  max-width: 100%;
  color: var(--text-color);
  font-weight: 600;
}

.unity-object-reference-placeholder {
  display: block;
  color: var(--text-secondary);
}

.unity-object-reference-clear {
  flex-shrink: 0;
  width: 22px;
  height: 22px;
  margin-right: 2px;
  padding: 0;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  line-height: 1;
  cursor: pointer;
}

.unity-object-reference-clear:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.unity-object-reference-dropdown {
  position: absolute;
  z-index: 40;
  left: 0;
  right: 0;
  top: calc(100% + 4px);
  max-height: 246px;
  overflow: auto;
  border: 1px solid var(--border-color);
  border-radius: 7px;
  background: var(--panel-bg);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.28);
}

.unity-object-reference-search {
  padding: 6px;
  border-bottom: 1px solid var(--border-color);
}

.unity-object-reference-search-input {
  width: 100%;
  min-width: 0;
  min-height: 24px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  font-family: var(--font-mono-identifier);
  box-sizing: border-box;
}

.unity-object-reference-search-input:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-object-reference-option,
.unity-object-reference-state {
  width: 100%;
  min-width: 0;
  padding: 6px 8px;
  border: 0;
  border-bottom: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  font-family: var(--font-mono-identifier);
  text-align: left;
  box-sizing: border-box;
}

.unity-object-reference-option {
  display: grid;
  gap: 2px;
  cursor: pointer;
}

.unity-object-reference-option:hover,
.unity-object-reference-option.highlighted {
  background: var(--hover-bg);
}

.unity-object-reference-option:last-child {
  border-bottom: 0;
}

.unity-object-reference-option-main {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.unity-object-reference-option-name,
.unity-object-reference-option-path,
.unity-object-reference-option-kind {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-object-reference-option-name {
  color: var(--text-color);
  font-weight: 600;
}

.unity-object-reference-option-kind {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-size: 10px;
}

.unity-object-reference-option-path {
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-object-reference-none .unity-object-reference-option-name,
.unity-object-reference-state {
  color: var(--text-secondary);
}
</style>
