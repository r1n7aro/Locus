<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import type { KnowledgeDocument } from "../../types";
import EmbeddedChatPane from "../chat/EmbeddedChatPane.vue";
import ModelEffortSelector from "../ModelEffortSelector.vue";
import { useEmbeddedChatSession } from "../../composables/useEmbeddedChatSession";
import { useDisplaySettings } from "../../composables/useDisplaySettings";
import { useSkills } from "../../composables/useSkills";
import { useAgentStore } from "../../stores/agent";
import { useModelStore } from "../../stores/model";
import { useProjectStore } from "../../stores/project";

const props = defineProps<{
  document: KnowledgeDocument;
}>();

const agentStore = useAgentStore();
const modelStore = useModelStore();
const projectStore = useProjectStore();
const { skillItems } = useSkills();
const { state: displaySettings } = useDisplaySettings();

const sessionKey = computed(() => `${projectStore.workingDir}::knowledge::${props.document.path}`);
const sessionTitle = computed(() => `Knowledge: ${props.document.title || props.document.path}`);
const manualKnowledgeAgentId = ref("");
const knowledgeDefaultAgentId = computed(() => {
  const defaultAgent = agentStore.agents.find((agent) => agent.isDefault);
  if (defaultAgent) return defaultAgent.id;
  return agentStore.agents[0]?.id || null;
});
const knowledgeAgentId = computed(() => {
  const manualSelectedId = manualKnowledgeAgentId.value.trim();
  if (manualSelectedId && agentStore.agents.some((agent) => agent.id === manualSelectedId)) {
    return manualSelectedId;
  }
  return knowledgeDefaultAgentId.value;
});

const placeholder = computed(() => (
  props.document.readOnly
    ? t("knowledge.chat.readOnlyPlaceholder")
    : t("knowledge.chat.placeholder")
));

const {
  inputText,
  messages,
  streamingText,
  thinkingText,
  streamingTextOrder,
  thinkingOrder,
  liveRenderParts,
  livePartStreams,
  isStreaming,
  isCompacting,
  isThinking,
  thinkingDuration,
  activeToolCalls,
  pendingQuestion,
  pendingToolConfirms,
  queuedFollowUp,
  errorMessage,
  sendComposerPayload,
  insertQueuedFollowUp,
  deleteQueuedFollowUp,
  cancel,
  answerQuestion,
  answerToolConfirm,
  answerAllToolConfirms,
  applyKnowledgeProposal,
  ignoreKnowledgeProposal,
  resetSession,
} = useEmbeddedChatSession({
  sessionKey,
  sessionType: "knowledge",
  sessionTitle,
  selectedModelId: computed(() => modelStore.selectedModelId),
  selectedAgentId: knowledgeAgentId,
  effort: computed(() => modelStore.effort),
  effortSupported: computed(() => modelStore.effortSupported),
  fastMode: computed(() => modelStore.effectiveCodexFastMode),
  // The current document is injected into the agent env by the backend
  // (knowledge focus), so user messages carry only what the user typed.
  knowledgeFocus: computed(() => ({
    docType: props.document.type,
    path: props.document.path,
  })),
  buildRequest(input) {
    return {
      text: input,
      displayText: input,
    };
  },
});

function handleSelectAgent(agentId: string) {
  manualKnowledgeAgentId.value = agentId;
  const agent = agentStore.agents.find((item) => item.id === agentId);
  const fallbackEffort = modelStore.hasUserDefaultEffort
    ? modelStore.defaultEffort
    : (agent?.defaultEffort ?? "none");
  modelStore.activateAgentPreference(agentId, fallbackEffort, true);
}
</script>

<template>
  <EmbeddedChatPane
    :messages="messages"
    :streaming-text="streamingText"
    :streaming-text-order="streamingTextOrder"
    :thinking-text="thinkingText"
    :thinking-order="thinkingOrder"
    :live-render-parts="liveRenderParts"
    :live-part-streams="livePartStreams"
    :is-streaming="isStreaming"
    :is-compacting="isCompacting"
    :is-thinking="isThinking"
    :thinking-duration="thinkingDuration"
    :active-tool-calls="activeToolCalls"
    :pending-question="pendingQuestion"
    :pending-tool-confirms="pendingToolConfirms"
    :queued-follow-up="queuedFollowUp"
    :tool-confirm-layout-key="sessionKey"
    :input-value="inputText"
    :placeholder="placeholder"
    :empty-title="t('knowledge.chat.emptyTitle')"
    :empty-hint="t('knowledge.chat.emptyHint')"
    :error-message="errorMessage"
    :send-label="t('knowledge.chat.send')"
    :cancel-label="t('common.cancel')"
    :user-label="t('knowledge.chat.user')"
    :assistant-label="t('knowledge.chat.assistant')"
    :thinking-label="t('knowledge.chat.thinking')"
    :waiting-label="t('chat.transcript.waiting')"
    :thought-duration-label="t('chat.transcript.thoughtDuration', '{0}')"
    :thought-moment-label="t('chat.transcript.thoughtMoment')"
    :running-label="t('knowledge.chat.running')"
    :selected-agent-id="knowledgeAgentId || ''"
    :skills="skillItems"
    enable-intent-badges
    show-user-images
    user-content-mode="asset"
    @update:input-value="inputText = $event"
    @send="sendComposerPayload"
    @insert-queued-follow-up="insertQueuedFollowUp"
    @delete-queued-follow-up="deleteQueuedFollowUp"
    @cancel="cancel"
    @clear="resetSession"
    @answer-question="answerQuestion"
    @answer-tool-confirm="answerToolConfirm"
    @answer-all-tool-confirms="answerAllToolConfirms"
    @apply-knowledge-proposal="applyKnowledgeProposal"
    @ignore-knowledge-proposal="ignoreKnowledgeProposal"
  >
    <template #composer-actions>
      <ModelEffortSelector
        :agents="displaySettings.showAgentSelector ? agentStore.agents : undefined"
        :selected-agent-id="knowledgeAgentId || ''"
        :models="modelStore.availableModels"
        :selected-id="modelStore.selectedModelId"
        :effort="modelStore.effort"
        :efforts="modelStore.availableEfforts"
        :effort-supported="modelStore.effortSupported"
        :fast-mode-enabled="modelStore.effectiveCodexFastMode"
        :fast-mode-available="modelStore.codexFastModeAvailable"
        :disabled="isStreaming"
        @select-agent="handleSelectAgent"
        @select-model="modelStore.selectModel"
        @select-effort="modelStore.selectEffort"
        @select-fast-mode="modelStore.selectCodexFastMode"
      />
    </template>
  </EmbeddedChatPane>
</template>
