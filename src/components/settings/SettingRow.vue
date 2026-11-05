<script setup lang="ts">
// One setting from the settings table, rendered by its type: bool → switch,
// enum → segmented control or dropdown, int → number input, text → text
// input, string list → comma separated input. Label and description come
// from `settings.item.<key>.label/.desc`; the row shows the experimental and
// restart badges, the pending write and the last write error. With `project`
// the row edits the focused project's setting instead of an app setting.
import { computed } from "vue";
import { t } from "../../i18n";
import { setSettingValue, useSetting, useSettingSchema } from "../../composables/useSettingsStore";
import { useProjectSetting } from "../../composables/useProjectSetting";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";

export interface SettingOption {
  value: string | number;
  label: string;
  hint?: string;
}

type Control = "switch" | "checkbox" | "segmented" | "dropdown" | "number" | "text" | "list";

const props = withDefaults(defineProps<{
  settingKey: string;
  label?: string;
  /** Overrides the catalog description; `false` hides it. */
  description?: string | false;
  options?: readonly SettingOption[];
  control?: Control;
  placeholder?: string;
  disabled?: boolean;
  experimental?: boolean;
  /** Number inputs only: unit label shown after the input. */
  unit?: string;
  /** A project-scope setting of the focused project. */
  project?: boolean;
  /** App setting whose effect targets the workspace it is changed from. */
  focusedWorkspace?: boolean;
}>(), {
  label: undefined,
  description: undefined,
  options: undefined,
  control: undefined,
  placeholder: "",
  disabled: false,
  experimental: false,
  unit: "",
  project: false,
  focusedWorkspace: false,
});

const schema = useSettingSchema(props.settingKey);
const setting = props.project
  ? useProjectSetting<unknown>(props.settingKey, (raw) => raw)
  : useSetting<unknown>(props.settingKey, (raw) => raw);

const value = computed(() => setting.value.value ?? schema.value?.default);
const ready = computed(() => setting.ready.value && value.value !== undefined);
const kind = computed(() => {
  if (schema.value) return schema.value.kind.type;
  if (props.options) return "enum";
  const current = value.value;
  if (typeof current === "boolean") return "bool";
  if (typeof current === "number") return "int";
  if (Array.isArray(current)) return "stringList";
  return "text";
});

const optionList = computed<SettingOption[]>(() => {
  if (props.options) return [...props.options];
  return (schema.value?.kind.values ?? []).map((item) => ({ value: item, label: item }));
});

const resolvedControl = computed<Control>(() => {
  if (props.control) return props.control;
  if (kind.value === "bool") return "switch";
  if (props.options || kind.value === "enum") return optionList.value.length <= 3 ? "segmented" : "dropdown";
  if (kind.value === "int") return "number";
  if (kind.value === "stringList") return "list";
  return "text";
});
const stacked = computed(() => resolvedControl.value === "text" || resolvedControl.value === "list");

const label = computed(() => props.label ?? t(`settings.item.${props.settingKey}.label`));
const description = computed(() => {
  if (props.description === false) return "";
  return props.description ?? t(`settings.item.${props.settingKey}.desc`);
});
const experimental = computed(() => props.experimental || Boolean(schema.value?.experimental));
const restart = computed(() => Boolean(schema.value?.restart));
const inactive = computed(() => props.disabled || !ready.value || setting.busy.value);

const min = computed(() => schema.value?.kind.min ?? Number.MIN_SAFE_INTEGER);
const max = computed(() => schema.value?.kind.max ?? Number.MAX_SAFE_INTEGER);

const workspaceContext = props.focusedWorkspace && !props.project ? useWorkspaceContextStore() : null;

function write(next: unknown): void {
  // Failures roll back and show inline; no extra notification.
  const saved = workspaceContext
    ? setSettingValue(props.settingKey, next, workspaceContext.focusedWorkspaceRef)
    : setting.set(next);
  void saved.catch(() => {});
}

function writeOption(raw: string): void {
  const option = optionList.value.find((item) => String(item.value) === raw);
  write(option ? option.value : raw);
}

const optionValue = computed(() => String(value.value ?? ""));
const segmentedOptions = computed(() => optionList.value.map((option) => ({
  value: String(option.value),
  label: option.label,
  hint: option.hint,
  disabled: inactive.value,
})));
const dropdownOptions = computed(() => optionList.value.map((option) => ({
  value: String(option.value),
  label: option.label,
  hint: option.hint,
})));

function commitNumber(event: Event): void {
  const input = event.target as HTMLInputElement;
  const parsed = Number.parseInt(input.value, 10);
  const current = typeof value.value === "number" ? value.value : 0;
  const next = Number.isFinite(parsed) ? Math.min(max.value, Math.max(min.value, parsed)) : current;
  input.value = String(next);
  if (next !== current) write(next);
}

function commitText(event: Event): void {
  const next = (event.target as HTMLInputElement).value;
  if (next !== value.value) write(next);
}

const listText = computed(() => (Array.isArray(value.value) ? value.value.join(", ") : ""));

function commitList(event: Event): void {
  const input = event.target as HTMLInputElement;
  const next = input.value.split(/[,，;；\n]+/).map((item) => item.trim()).filter(Boolean);
  if (JSON.stringify(next) !== JSON.stringify(value.value)) write(next);
  input.value = next.join(", ");
}

// Focused number inputs step on wheel in Chromium; scrolling the page must
// not change saved values.
function guardWheel(event: WheelEvent): void {
  if (event.target === document.activeElement) event.preventDefault();
}
</script>

<template>
  <div
    class="setting-row"
    :class="{ stacked, busy: setting.busy.value, 'has-error': setting.error.value }"
    :data-setting-key="settingKey"
    :aria-busy="!ready || setting.busy.value"
  >
    <div class="setting-row-text">
      <div class="setting-row-label">
        <span>{{ label }}</span>
        <span v-if="experimental" class="settings-badge settings-badge-experimental">
          {{ t("settings.badge.experimental") }}
        </span>
        <span v-if="restart" class="settings-badge">{{ t("settings.badge.restart") }}</span>
      </div>
      <div v-if="description" class="setting-row-desc">{{ description }}</div>
      <div v-if="setting.error.value" class="setting-row-error" role="alert">{{ setting.error.value }}</div>
    </div>
    <div class="setting-row-control">
      <template v-if="!ready">
        <span class="setting-row-placeholder" aria-hidden="true" />
      </template>
      <BaseSwitch
        v-else-if="resolvedControl === 'switch'"
        :model-value="value === true"
        :disabled="inactive"
        :aria-label="label"
        @update:model-value="write"
      />
      <BaseCheckbox
        v-else-if="resolvedControl === 'checkbox'"
        :model-value="value === true"
        :disabled="inactive"
        :aria-label="label"
        @update:model-value="write"
      />
      <BaseSegmented
        v-else-if="resolvedControl === 'segmented'"
        :model-value="optionValue"
        :options="segmentedOptions"
        :aria-label="label"
        size="sm"
        @update:model-value="writeOption"
      />
      <BaseDropdown
        v-else-if="resolvedControl === 'dropdown'"
        class="setting-row-dropdown"
        :model-value="optionValue"
        :options="dropdownOptions"
        :aria-label="label"
        :disabled="inactive"
        size="sm"
        menu-align="end"
        teleport
        @update:model-value="writeOption"
      />
      <template v-else-if="resolvedControl === 'number'">
        <input
          class="setting-row-input setting-row-number"
          type="number"
          step="1"
          :min="schema?.kind.min"
          :max="schema?.kind.max"
          :value="value"
          :disabled="inactive"
          :aria-label="label"
          @wheel="guardWheel"
          @change="commitNumber"
          @keydown.enter="($event.target as HTMLInputElement).blur()"
        />
        <span v-if="unit" class="setting-row-unit">{{ unit }}</span>
      </template>
      <input
        v-else-if="resolvedControl === 'list'"
        class="setting-row-input setting-row-text-input"
        type="text"
        spellcheck="false"
        autocomplete="off"
        :value="listText"
        :placeholder="placeholder"
        :disabled="inactive"
        :aria-label="label"
        @change="commitList"
        @keydown.enter="($event.target as HTMLInputElement).blur()"
      />
      <input
        v-else
        class="setting-row-input setting-row-text-input"
        type="text"
        spellcheck="false"
        autocomplete="off"
        :maxlength="schema?.kind.maxLength"
        :value="typeof value === 'string' ? value : ''"
        :placeholder="placeholder"
        :disabled="inactive"
        :aria-label="label"
        @change="commitText"
        @keydown.enter="($event.target as HTMLInputElement).blur()"
      />
    </div>
  </div>
</template>

<style scoped>
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  max-width: 760px;
  padding: 10px 0;
}

.setting-row + .setting-row {
  border-top: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
}

.setting-row.stacked {
  flex-direction: column;
  align-items: stretch;
  gap: 8px;
}

.setting-row-text {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.setting-row-label {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  color: var(--text-color);
  font-size: 13px;
}

.setting-row-desc {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.setting-row-error {
  color: var(--status-danger-fg);
  font-size: 12px;
  line-height: 1.45;
}

.setting-row-control {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.setting-row.stacked .setting-row-control {
  flex-shrink: 1;
}

.setting-row-placeholder {
  width: 34px;
  height: 18px;
}

.setting-row-input {
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.setting-row-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.setting-row-input:disabled {
  opacity: 0.6;
}

.setting-row-number {
  width: 80px;
  text-align: center;
  appearance: textfield;
  -moz-appearance: textfield;
}

.setting-row-text-input {
  width: 100%;
  max-width: 560px;
}

.setting-row-unit {
  color: var(--text-secondary);
  font-size: 12px;
}

.setting-row-dropdown {
  min-width: 140px;
}
</style>
