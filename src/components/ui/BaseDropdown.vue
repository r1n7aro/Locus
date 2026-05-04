<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";

export interface DropdownOption {
  value: string;
  label: string;
  hint?: string;
  disabled?: boolean;
}

const props = withDefaults(defineProps<{
  modelValue: string;
  options: DropdownOption[];
  selectedLabel?: string;
  size?: "sm" | "md";
  menuAlign?: "start" | "end";
  placeholder?: string;
  ariaLabel?: string;
  disabled?: boolean;
}>(), {
  selectedLabel: "",
  size: "sm",
  menuAlign: "end",
  placeholder: "",
  ariaLabel: "",
  disabled: false,
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

const open = ref(false);
const rootRef = ref<HTMLElement | null>(null);
const triggerRef = ref<HTMLButtonElement | null>(null);
const listboxRef = ref<HTMLElement | null>(null);
const activeIndex = ref(-1);
const listboxId = `dropdown-${Math.random().toString(36).slice(2, 10)}`;

const selectedOption = computed(() =>
  props.options.find((option) => option.value === props.modelValue) ?? null,
);
const enabledOptions = computed(() => props.options.filter((option) => !option.disabled));
const activeDescendant = computed(() => {
  const option = props.options[activeIndex.value];
  return option ? `${listboxId}-option-${option.value}` : undefined;
});

function toggleOpen() {
  if (props.disabled) return;
  if (open.value) {
    close();
    return;
  }
  openMenu();
}

function close() {
  open.value = false;
  activeIndex.value = -1;
}

function select(value: string, disabled?: boolean) {
  if (disabled || value === props.modelValue) {
    close();
    return;
  }
  emit("update:modelValue", value);
  close();
  triggerRef.value?.focus();
}

function onDocumentClick(event: MouseEvent) {
  if (rootRef.value && !rootRef.value.contains(event.target as Node)) {
    close();
  }
}

function focusOptionAt(index: number) {
  const option = props.options[index];
  if (!option || option.disabled) return;
  activeIndex.value = index;
}

function firstEnabledIndex(): number {
  return props.options.findIndex((option) => !option.disabled);
}

function selectedEnabledIndex(): number {
  const index = props.options.findIndex((option) => option.value === props.modelValue && !option.disabled);
  return index >= 0 ? index : firstEnabledIndex();
}

function moveActive(step: 1 | -1) {
  if (!enabledOptions.value.length) return;
  const enabledIndexes = props.options
    .map((option, index) => (!option.disabled ? index : -1))
    .filter((index) => index >= 0);
  const currentPos = enabledIndexes.indexOf(activeIndex.value);
  const startPos = currentPos >= 0 ? currentPos : enabledIndexes.indexOf(selectedEnabledIndex());
  const nextPos = (startPos + step + enabledIndexes.length) % enabledIndexes.length;
  activeIndex.value = enabledIndexes[nextPos];
}

function openMenu() {
  open.value = true;
  activeIndex.value = selectedEnabledIndex();
}

function onKeydown(event: KeyboardEvent) {
  if (props.disabled) return;
  if (!open.value && (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Enter" || event.key === " ")) {
    event.preventDefault();
    openMenu();
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    close();
    triggerRef.value?.focus();
    return;
  }
  if (!open.value) return;
  if (event.key === "ArrowDown") {
    event.preventDefault();
    moveActive(1);
    return;
  }
  if (event.key === "ArrowUp") {
    event.preventDefault();
    moveActive(-1);
    return;
  }
  if (event.key === "Home") {
    event.preventDefault();
    activeIndex.value = firstEnabledIndex();
    return;
  }
  if (event.key === "End") {
    event.preventDefault();
    const enabledIndexes = props.options
      .map((option, index) => (!option.disabled ? index : -1))
      .filter((index) => index >= 0);
    activeIndex.value = enabledIndexes[enabledIndexes.length - 1] ?? -1;
    return;
  }
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    const option = props.options[activeIndex.value];
    if (option) select(option.value, option.disabled);
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick, true);
});

onUnmounted(() => {
  document.removeEventListener("click", onDocumentClick, true);
});
</script>

<template>
  <div ref="rootRef" class="base-dropdown" :class="[`size-${size}`]" @keydown.capture="onKeydown">
    <button
      ref="triggerRef"
      class="base-dropdown-trigger"
      :class="{ disabled }"
      type="button"
      aria-haspopup="listbox"
      :aria-expanded="open"
      :aria-label="ariaLabel || undefined"
      :aria-controls="open ? listboxId : undefined"
      :aria-activedescendant="open ? activeDescendant : undefined"
      :disabled="disabled"
      @click.stop="toggleOpen"
    >
      <span class="base-dropdown-value">{{ selectedLabel || selectedOption?.label || placeholder }}</span>
      <span class="base-dropdown-chevron" :class="{ open }">&#9662;</span>
    </button>

    <Transition name="dropdown">
      <div
        v-if="open"
        :id="listboxId"
        ref="listboxRef"
        class="base-dropdown-menu"
        :class="[`align-${menuAlign}`]"
        role="listbox"
        tabindex="-1"
      >
        <button
          v-for="(option, index) in options"
          :key="option.value"
          :id="`${listboxId}-option-${option.value}`"
          type="button"
          class="base-dropdown-item"
          :class="{ active: modelValue === option.value, focused: activeIndex === index }"
          role="option"
          :aria-selected="modelValue === option.value"
          :disabled="option.disabled"
          @click="select(option.value, option.disabled)"
          @focus="focusOptionAt(index)"
          @mousemove="focusOptionAt(index)"
        >
          <span class="base-dropdown-item-label">{{ option.label }}</span>
          <span v-if="option.hint" class="base-dropdown-item-hint">{{ option.hint }}</span>
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.base-dropdown {
  position: relative;
  min-width: 0;
}

.base-dropdown-trigger {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-width: 110px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--bg-color));
  color: var(--text-color);
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.base-dropdown-trigger:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

.base-dropdown-trigger.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.base-dropdown-trigger:focus-visible {
  outline: none;
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 18%, transparent);
  border-color: var(--accent-color);
}

.base-dropdown-value {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
}

.base-dropdown-chevron {
  flex-shrink: 0;
  font-size: 10px;
  color: var(--text-secondary);
  transition: transform 0.15s ease;
}

.base-dropdown-chevron.open {
  transform: rotate(180deg);
}

.base-dropdown-menu {
  position: absolute;
  top: calc(100% + 6px);
  min-width: max(220px, 100%);
  width: max-content;
  max-width: min(720px, calc(100vw - 32px));
  padding: 4px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--elevated-bg, var(--panel-bg));
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.22);
  z-index: 40;
}

.base-dropdown-menu.align-start {
  left: 0;
}

.base-dropdown-menu.align-end {
  right: 0;
}

.base-dropdown-item {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  gap: 2px;
  border: none;
  border-radius: 6px;
  background: transparent;
  text-align: left;
  color: var(--text-color);
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}

.base-dropdown-item:hover:not(:disabled) {
  background: var(--hover-bg);
}

.base-dropdown-item.active {
  background: var(--accent-soft);
  color: var(--accent-color);
}

.base-dropdown-item.focused:not(.active) {
  background: var(--hover-bg);
}

.base-dropdown-item:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.base-dropdown-item-label {
  font-size: 12px;
  font-weight: 500;
  min-width: 0;
  max-width: 100%;
  overflow-wrap: anywhere;
}

.base-dropdown-item-hint {
  min-width: 0;
  max-width: 100%;
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.4;
  white-space: normal;
  overflow-wrap: anywhere;
}

.base-dropdown-item.active .base-dropdown-item-hint {
  color: color-mix(in srgb, var(--accent-color) 68%, var(--text-secondary) 32%);
}

.size-sm .base-dropdown-trigger {
  min-height: 28px;
  padding: 0 10px;
  font-size: 12px;
}

.size-sm .base-dropdown-item {
  padding: 8px 10px;
}

.size-md .base-dropdown-trigger {
  min-height: 32px;
  padding: 0 12px;
  font-size: 13px;
}

.size-md .base-dropdown-item {
  padding: 9px 12px;
}

.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
