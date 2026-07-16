<script setup lang="ts">
import { computed } from "vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import { t } from "../../i18n";
import { defaultReasoningParamFormat } from "../../services/modelCatalog";
import type {
  ApiFormat,
  CustomProviderModel,
  EffortLevel,
  ReasoningParamFormat,
  ReasoningReplayField,
} from "../../types";

const props = defineProps<{
  /** Draft row, mutated in place (same contract as the provider draft itself). */
  model: CustomProviderModel;
  apiFormat: ApiFormat;
  saving?: boolean;
}>();

const EFFORT_OPTIONS = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Med" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
  { value: "max", label: "Max" },
] satisfies Array<{ value: EffortLevel; label: string }>;

const FORMAT_LABEL_KEYS: Record<ReasoningParamFormat, string> = {
  none: "settings.custom.reasoningNone",
  openai_chat_reasoning_effort: "settings.custom.reasoningOpenaiChat",
  openai_chat_enable_thinking: "settings.custom.reasoningEnableThinking",
  openai_chat_thinking_type: "settings.custom.reasoningThinkingType",
  openai_responses_reasoning_effort: "settings.custom.reasoningOpenaiResponses",
  anthropic_thinking: "settings.custom.reasoningAnthropic",
};

/** Reasoning params that make sense per wire format; a stale cross-format
 *  value is appended so an edited legacy row never renders a blank select. */
const FORMATS_BY_API: Record<ApiFormat, ReasoningParamFormat[]> = {
  openai_chat: [
    "none",
    "openai_chat_reasoning_effort",
    "openai_chat_enable_thinking",
    "openai_chat_thinking_type",
  ],
  openai_responses: ["none", "openai_responses_reasoning_effort"],
  anthropic_messages: ["none", "anthropic_thinking"],
};

const REPLAY_FIELD_OPTIONS = [
  { value: "", label: t("settings.custom.replayFieldAuto") },
  { value: "reasoning_content", label: "reasoning_content" },
  { value: "reasoning_details", label: "reasoning_details" },
  { value: "reasoning", label: "reasoning" },
] satisfies Array<{ value: ReasoningReplayField | ""; label: string }>;

const BETA_FLAG_OPTIONS = [
  { flag: "context-1m-2025-08-07", descKey: "settings.custom.betaContext1m" },
  { flag: "interleaved-thinking-2025-05-14", descKey: "settings.custom.betaInterleavedThinking" },
  { flag: "prompt-caching-scope-2026-01-05", descKey: "settings.custom.betaPromptCaching" },
];

/** null means "derive on save"; show the derived value instead of a blank. */
const effectiveReasoningFormat = computed<ReasoningParamFormat>(
  () => props.model.reasoningParamFormat ?? defaultReasoningParamFormat(props.apiFormat),
);

const reasoningFormatOptions = computed(() => {
  const allowed = [...FORMATS_BY_API[props.apiFormat]];
  if (!allowed.includes(effectiveReasoningFormat.value)) {
    allowed.push(effectiveReasoningFormat.value);
  }
  return allowed.map((value) => ({ value, label: t(FORMAT_LABEL_KEYS[value]) }));
});

function updateReasoningFormat(value: string) {
  props.model.reasoningParamFormat = value as ReasoningParamFormat;
}

function setReasoningEffortEnabled(effort: EffortLevel, enabled: boolean) {
  const model = props.model;
  const list = model.supportedReasoningEfforts ?? (model.supportedReasoningEfforts = []);
  const idx = list.indexOf(effort);
  if (enabled && idx < 0) list.push(effort);
  else if (!enabled && idx >= 0) list.splice(idx, 1);
}

function setBetaFlagEnabled(flag: string, enabled: boolean) {
  const model = props.model;
  const list = model.betaFlags ?? (model.betaFlags = []);
  const idx = list.indexOf(flag);
  if (enabled && idx < 0) list.push(flag);
  else if (!enabled && idx >= 0) list.splice(idx, 1);
}

function updateReplayField(value: string) {
  props.model.reasoningReplayField = value === "" ? null : (value as ReasoningReplayField);
}
</script>

<template>
  <!-- One grid for every section: the label column is shared across the whole
       form, so controls line up on a single edge; section titles span both
       columns. Rows are plain label/control pairs — no nested blocks. -->
  <div class="model-form">
    <div class="form-section-title">{{ t("settings.custom.formSectionBasic") }}</div>

    <span class="form-row-label">{{ t("settings.custom.apiModel") }}</span>
    <div class="form-control">
      <input
        v-model="model.apiModel"
        class="form-input mono-input input-md"
        type="text"
        :disabled="saving"
        :placeholder="t('settings.custom.apiModelPlaceholder')"
      />
    </div>

    <span class="form-row-label">{{ t("settings.custom.modelDisplayName") }}</span>
    <div class="form-control">
      <input
        v-model="model.name"
        class="form-input input-md"
        type="text"
        :disabled="saving"
        :placeholder="model.apiModel"
      />
    </div>

    <span class="form-row-label">{{ t("settings.custom.contextLength") }}</span>
    <div class="form-control">
      <input
        v-model.number="model.contextLength"
        class="form-input mono-input input-sm"
        type="number"
        :disabled="saving"
        min="1024"
        step="1024"
        placeholder="256000"
      />
    </div>

    <span class="form-row-label">{{ t("settings.custom.imageUnderstanding") }}</span>
    <div class="form-control form-control-inline">
      <BaseCheckbox
        v-model="model.supportsVision"
        :disabled="saving"
        :aria-label="t('settings.custom.imageUnderstanding')"
      />
    </div>

    <div class="form-section-title">{{ t("settings.custom.formSectionReasoning") }}</div>

    <span class="form-row-label">{{ t("settings.custom.reasoningFormat") }}</span>
    <div class="form-control">
      <BaseDropdown
        class="select-md"
        size="md"
        menu-align="start"
        :model-value="effectiveReasoningFormat"
        :options="reasoningFormatOptions"
        :aria-label="t('settings.custom.reasoningFormat')"
        :disabled="saving"
        @update:model-value="updateReasoningFormat"
      />
    </div>

    <template v-if="effectiveReasoningFormat !== 'none'">
      <span class="form-row-label">{{ t("settings.custom.reasoningEfforts") }}</span>
      <div class="form-control form-control-inline">
        <div class="form-effort-options">
          <div v-for="option in EFFORT_OPTIONS" :key="option.value" class="form-option-row">
            <BaseCheckbox
              :disabled="saving"
              :model-value="model.supportedReasoningEfforts?.includes(option.value) ?? false"
              :aria-label="option.label"
              @update:model-value="setReasoningEffortEnabled(option.value, $event)"
            />
            <span class="form-option-name mono">{{ option.label }}</span>
          </div>
        </div>
      </div>
    </template>

    <template v-if="apiFormat === 'openai_chat'">
      <span class="form-row-label">{{ t("settings.custom.replayReasoningContent") }}</span>
      <div class="form-control form-control-inline">
        <BaseCheckbox
          :model-value="model.replayReasoningContent === true"
          :disabled="saving"
          :aria-label="t('settings.custom.replayReasoningContent')"
          @update:model-value="model.replayReasoningContent = $event"
        />
      </div>
      <template v-if="model.replayReasoningContent === true">
        <span class="form-row-label">{{ t("settings.custom.replayField") }}</span>
        <div class="form-control">
          <BaseDropdown
            class="select-md"
            size="md"
            menu-align="start"
            :model-value="model.reasoningReplayField ?? ''"
            :options="REPLAY_FIELD_OPTIONS"
            :aria-label="t('settings.custom.replayField')"
            :disabled="saving"
            @update:model-value="updateReplayField"
          />
        </div>
      </template>
    </template>

    <template v-if="apiFormat === 'anthropic_messages'">
      <div class="form-section-title">{{ t("settings.custom.formSectionAnthropic") }}</div>

      <span class="form-row-label form-row-label-top">{{ t("settings.custom.betaFlags") }}</span>
      <div class="form-control">
        <div class="form-options-list">
          <div v-for="beta in BETA_FLAG_OPTIONS" :key="beta.flag" class="form-option-row">
            <BaseCheckbox
              :disabled="saving"
              :model-value="model.betaFlags?.includes(beta.flag) ?? false"
              :aria-label="beta.flag"
              @update:model-value="setBetaFlagEnabled(beta.flag, $event)"
            />
            <div class="form-option-copy">
              <span class="form-option-name mono">{{ beta.flag }}</span>
              <span class="form-option-desc">{{ t(beta.descKey) }}</span>
            </div>
          </div>
        </div>
      </div>

      <span class="form-row-label form-row-label-top">{{ t("settings.custom.serverTools") }}</span>
      <div class="form-control">
        <div class="form-option-row">
          <BaseCheckbox
            :disabled="saving"
            :model-value="model.serverTools?.webSearch ?? false"
            aria-label="web_search"
            @update:model-value="model.serverTools = { ...(model.serverTools ?? {}), webSearch: $event }"
          />
          <div class="form-option-copy">
            <span class="form-option-name mono">web_search</span>
            <span class="form-option-desc">{{ t("settings.custom.serverToolWebSearch") }}</span>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
/* Label column left, controls right; one grid for the whole form so the
 * column hugs the widest label anywhere and every control shares one edge. */
.model-form {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 12px 16px;
  align-items: start;
}

.form-section-title {
  grid-column: 1 / -1;
  margin-top: 6px;
  font-size: 10.5px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: color-mix(in srgb, var(--accent-color) 45%, var(--text-secondary) 55%);
  user-select: none;
}

.form-section-title:first-child {
  margin-top: 0;
}

.form-row-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  text-align: right;
  white-space: nowrap;
  line-height: 30px;
  min-width: 0;
}

.form-row-label-top {
  line-height: 18px;
}

.form-control {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}

/* Checkbox/option rows: center against the 30px label line height. */
.form-control-inline {
  flex-direction: row;
  align-items: center;
  min-height: 30px;
}

.form-input {
  width: 100%;
  min-width: 0;
  padding: 7px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  box-sizing: border-box;
  transition: border-color 0.15s ease, background 0.15s ease;
}

/* Controls sized to their content instead of stretching across the pane. */
.input-md {
  max-width: 340px;
}

.input-sm {
  max-width: 160px;
}

.select-md {
  max-width: 320px;
}

.mono-input {
  font-family: var(--font-mono-editor);
}

.form-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.form-input:disabled {
  opacity: 0.65;
  cursor: not-allowed;
}

.form-input[type="number"] {
  appearance: textfield;
  -moz-appearance: textfield;
}

.form-input[type="number"]::-webkit-inner-spin-button,
.form-input[type="number"]::-webkit-outer-spin-button {
  margin: 0;
  -webkit-appearance: none;
}

.form-effort-options {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px 16px;
}

.form-options-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.form-option-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  min-height: 18px;
}

.form-option-copy {
  min-width: 0;
  display: flex;
  flex-direction: row;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 2px 8px;
}

.form-option-name {
  font-size: 12px;
  line-height: 18px;
  color: var(--text-color);
}

.form-option-desc {
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.4;
}

.mono {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  white-space: nowrap;
}
</style>
