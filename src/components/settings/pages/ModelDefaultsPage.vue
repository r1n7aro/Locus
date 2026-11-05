<script setup lang="ts">
import { onMounted, ref } from "vue";
import { t } from "../../../i18n";
import { getModelDefaults, saveModelDefaults } from "../../../services/model";
import { normalizeAppError } from "../../../services/errors";
import { normalizeSubagentModelChoices } from "../../../utils/subagentModels";
import { useModelStore } from "../../../stores/model";
import { useNotificationStore } from "../../../stores/notification";
import { useSettingsFeedbackStore } from "../../../stores/settingsFeedback";
import type { AgentInfo, ModelDefaults, ModelOption } from "../../../types";
import ModelDefaultsPanel from "../ModelDefaults.vue";

defineProps<{
  allModels: ModelOption[];
  agents: AgentInfo[];
  subagents: AgentInfo[];
}>();

const modelStore = useModelStore();
const feedback = useSettingsFeedbackStore();

function normalize(defaults: Partial<ModelDefaults> | null | undefined): ModelDefaults {
  return {
    mainModel: defaults?.mainModel ?? "",
    planModel: defaults?.planModel ?? "",
    subagentModels: normalizeSubagentModelChoices(defaults?.subagentModels),
    subagentEfforts: defaults?.subagentEfforts ?? {},
    subagentFastModes: defaults?.subagentFastModes ?? {},
  };
}

// Start from what the app already uses, then read the saved values.
const modelDefaults = ref<ModelDefaults>(normalize(modelStore.modelDefaults));

onMounted(async () => {
  try {
    modelDefaults.value = normalize(await getModelDefaults());
  } catch { /* keep the app's copy */ }
});

async function save() {
  try {
    await saveModelDefaults(modelDefaults.value);
    modelStore.applyModelDefaults(modelDefaults.value);
    feedback.flashSuccess(t("settings.models.saved"));
  } catch (e) {
    const err = normalizeAppError(e);
    useNotificationStore().addNotice("error", t("settings.models.saveFailed", err.message), {
      code: err.code,
      operation: "saveModelDefaults",
    });
  }
}
</script>

<template>
  <ModelDefaultsPanel
    :model-defaults="modelDefaults"
    :all-models="allModels"
    :agents="agents"
    :subagents="subagents"
    :model-save-msg="feedback.success"
    @update:model-defaults="modelDefaults = $event"
    @save="save"
  />
</template>
