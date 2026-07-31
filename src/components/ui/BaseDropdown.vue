<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch, type CSSProperties } from "vue";

export interface DropdownOption {
  value: string;
  label: string;
  hint?: string;
  disabled?: boolean;
  /** Section header; consecutive options sharing a group render under one header. */
  group?: string;
  /** Inline style for the option label (e.g. font preview). */
  labelStyle?: CSSProperties;
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
  /** Render the menu on <body> with fixed positioning so it escapes
   *  overflow-clipping ancestors (scroll containers, embedded panels). */
  teleport?: boolean;
}>(), {
  selectedLabel: "",
  size: "sm",
  menuAlign: "end",
  placeholder: "",
  ariaLabel: "",
  disabled: false,
  teleport: false,
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  open: [];
}>();

const open = ref(false);
const rootRef = ref<HTMLElement | null>(null);
const triggerRef = ref<HTMLButtonElement | null>(null);
const listboxRef = ref<HTMLElement | null>(null);
const activeIndex = ref(-1);
const menuFixedStyle = ref<CSSProperties>({});
const listboxId = `dropdown-${Math.random().toString(36).slice(2, 10)}`;

const selectedOption = computed(() =>
  props.options.find((option) => option.value === props.modelValue) ?? null,
);
const enabledOptions = computed(() => props.options.filter((option) => !option.disabled));
const activeDescendant = computed(() => {
  const option = props.options[activeIndex.value];
  return option ? `${listboxId}-option-${option.value}` : undefined;
});

function isGroupStart(index: number): boolean {
  const option = props.options[index];
  if (!option?.group) return false;
  return index === 0 || props.options[index - 1]?.group !== option.group;
}

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
  const target = event.target as Node;
  if (rootRef.value?.contains(target)) return;
  if (listboxRef.value?.contains(target)) return;
  close();
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

// Keep in sync with the .base-dropdown-menu max-height rule below.
const MENU_MAX_HEIGHT = 420;
const MENU_GAP = 6;
const MENU_MARGIN = 8;

function repositionMenu() {
  if (!props.teleport) return;
  const trigger = triggerRef.value;
  const menu = listboxRef.value;
  if (!trigger || !menu) return;
  const rect = trigger.getBoundingClientRect();
  const viewport = { width: window.innerWidth, height: window.innerHeight };

  // The menu never renders narrower than the trigger (minWidth below), so
  // horizontal math must use the floored width, not the raw measurement.
  const menuWidth = Math.max(menu.getBoundingClientRect().width, rect.width);
  const borderChrome = menu.offsetHeight - menu.clientHeight;
  const desiredHeight = Math.min(menu.scrollHeight + borderChrome, MENU_MAX_HEIGHT);

  // Pick the roomier side and shrink the menu into it instead of letting it
  // cover the trigger or run past the viewport edge.
  const spaceBelow = viewport.height - rect.bottom - MENU_GAP - MENU_MARGIN;
  const spaceAbove = rect.top - MENU_GAP - MENU_MARGIN;
  const placeBelow = spaceBelow >= desiredHeight || spaceBelow >= spaceAbove;
  const maxHeight = Math.max(80, Math.min(desiredHeight, placeBelow ? spaceBelow : spaceAbove));
  const height = Math.min(desiredHeight, maxHeight);
  const top = placeBelow ? rect.bottom + MENU_GAP : rect.top - MENU_GAP - height;

  const rawLeft = props.menuAlign === "end" ? rect.right - menuWidth : rect.left;
  const maxLeft = Math.max(MENU_MARGIN, viewport.width - menuWidth - MENU_MARGIN);
  const left = Math.min(Math.max(rawLeft, MENU_MARGIN), maxLeft);

  menuFixedStyle.value = {
    position: "fixed",
    top: `${Math.max(MENU_MARGIN, top)}px`,
    left: `${left}px`,
    minWidth: `${rect.width}px`,
    maxHeight: `${maxHeight}px`,
  };
}

function onWindowScroll(event: Event) {
  if (!open.value) return;
  const target = event.target as Node | null;
  if (target && listboxRef.value && target !== (document as unknown as Node) && listboxRef.value.contains(target)) {
    return;
  }
  repositionMenu();
}

function onWindowResize() {
  if (open.value) repositionMenu();
}

function openMenu() {
  const wasOpen = open.value;
  open.value = true;
  activeIndex.value = selectedEnabledIndex();
  if (props.teleport) {
    const triggerWidth = triggerRef.value?.getBoundingClientRect().width ?? 0;
    menuFixedStyle.value = {
      position: "fixed",
      top: "0px",
      left: "0px",
      minWidth: `${triggerWidth}px`,
      visibility: "hidden",
    };
    nextTick(() => {
      repositionMenu();
      scrollActiveIntoView();
    });
  } else {
    nextTick(scrollActiveIntoView);
  }
  if (!wasOpen) {
    emit("open");
  }
}

function scrollActiveIntoView() {
  if (!open.value) return;
  const option = props.options[activeIndex.value];
  if (!option) return;
  document.getElementById(`${listboxId}-option-${option.value}`)?.scrollIntoView({ block: "nearest" });
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

watch(activeIndex, () => {
  if (open.value) nextTick(scrollActiveIntoView);
});

watch(open, (isOpen) => {
  if (!props.teleport) return;
  if (isOpen) {
    window.addEventListener("scroll", onWindowScroll, true);
    window.addEventListener("resize", onWindowResize);
  } else {
    window.removeEventListener("scroll", onWindowScroll, true);
    window.removeEventListener("resize", onWindowResize);
  }
});

onMounted(() => {
  document.addEventListener("click", onDocumentClick, true);
});

onUnmounted(() => {
  document.removeEventListener("click", onDocumentClick, true);
  window.removeEventListener("scroll", onWindowScroll, true);
  window.removeEventListener("resize", onWindowResize);
});
</script>

<template>
  <div ref="rootRef" class="base-dropdown" :class="[`size-${size}`, { open }]" @keydown.capture="onKeydown">
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

    <Teleport to="body" :disabled="!teleport">
      <Transition name="dropdown">
        <div
          v-if="open"
          :id="listboxId"
          ref="listboxRef"
          class="base-dropdown-menu"
          :class="[`align-${menuAlign}`, `size-${size}`, { teleported: teleport }]"
          :style="teleport ? menuFixedStyle : undefined"
          role="listbox"
          tabindex="-1"
        >
          <template v-for="(option, index) in options" :key="option.value">
            <div v-if="isGroupStart(index)" class="base-dropdown-group-label">{{ option.group }}</div>
            <button
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
              <span class="base-dropdown-item-label" :style="option.labelStyle">{{ option.label }}</span>
              <span v-if="option.hint" class="base-dropdown-item-hint">{{ option.hint }}</span>
            </button>
          </template>
        </div>
      </Transition>
    </Teleport>
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
  box-sizing: border-box;
  min-width: max(220px, 100%);
  width: max-content;
  max-width: min(720px, calc(100vw - 32px));
  /* 420 mirrors MENU_MAX_HEIGHT in the script. */
  max-height: min(420px, calc(100vh - 24px));
  overflow-y: auto;
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

.base-dropdown-menu.teleported {
  min-width: 0;
  left: auto;
  right: auto;
  top: auto;
  z-index: 2000;
}

.base-dropdown-group-label {
  padding: 6px 10px 2px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  opacity: 0.7;
}

.base-dropdown-group-label:not(:first-child) {
  margin-top: 4px;
  border-top: 1px solid var(--border-color);
  padding-top: 8px;
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

.base-dropdown-menu.size-sm .base-dropdown-item {
  padding: 8px 10px;
}

.size-md .base-dropdown-trigger {
  min-height: 32px;
  padding: 0 12px;
  font-size: 13px;
}

.base-dropdown-menu.size-md .base-dropdown-item {
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
