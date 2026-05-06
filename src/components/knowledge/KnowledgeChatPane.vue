<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import type { KnowledgeDocument, KnowledgeEditMode, KnowledgeDocumentType } from "../../types";
import EmbeddedChatPane from "../chat/EmbeddedChatPane.vue";
import AgentSelector from "../AgentSelector.vue";
import ModelEffortSelector from "../ModelEffortSelector.vue";
import { useEmbeddedChatSession } from "../../composables/useEmbeddedChatSession";
import { useSkills } from "../../composables/useSkills";
import { useAgentStore } from "../../stores/agent";
import { useModelStore } from "../../stores/model";
import { useProjectStore } from "../../stores/project";
import { getKnowledgeEditMode } from "./knowledgeEditMode";

const BODY_CONTEXT_LIMIT = 6000;

const props = defineProps<{
  document: KnowledgeDocument;
}>();

const agentStore = useAgentStore();
const modelStore = useModelStore();
const projectStore = useProjectStore();
const { skillItems } = useSkills();

const sessionKey = computed(() => `${projectStore.workingDir}::knowledge::${props.document.path}`);
const sessionTitle = computed(() => `Knowledge: ${props.document.title || props.document.path}`);
const editMode = computed<KnowledgeEditMode>(() => getKnowledgeEditMode(props.document));
const manualKnowledgeAgentId = ref("");
const knowledgeDefaultAgentId = computed(() => {
  if (agentStore.agents.some((agent) => agent.id === "knowledge")) return "knowledge";
  const selectedAgentId = agentStore.selectedAgentId.trim();
  if (selectedAgentId && agentStore.agents.some((agent) => agent.id === selectedAgentId)) {
    return selectedAgentId;
  }
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
  isStreaming,
  isThinking,
  thinkingDuration,
  activeToolCalls,
  pendingQuestion,
  pendingToolConfirms,
  errorMessage,
  send,
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
  buildRequest(input) {
    const summary = props.document.summaryEnabled ? props.document.summary?.trim() : "";
    const rules = props.document.explicitMaintenanceRules ? props.document.maintenanceRules?.trim() : "";
    const body = trimmedContext(props.document.body?.trim() ?? "", BODY_CONTEXT_LIMIT);
    const lines = [
      t("knowledge.chat.request.intro"),
      t("knowledge.chat.request.title", props.document.title || props.document.path),
      t("knowledge.chat.request.path", props.document.path),
      t("knowledge.chat.request.type", typeLabel(props.document.type)),
      t("knowledge.chat.request.scope", scopeLabel(props.document)),
      t("knowledge.chat.request.editMode", editModeLabel(editMode.value)),
    ];

    if (summary) {
      lines.push(t("knowledge.chat.request.summaryHeader"));
      lines.push(summary);
    }
    if (rules) {
      lines.push(t("knowledge.chat.request.rulesHeader"));
      lines.push(rules);
    }
    if (body) {
      lines.push(t("knowledge.chat.request.bodyHeader"));
      lines.push(body);
    }

    lines.push(t("knowledge.chat.request.requirementsHeader"));
    lines.push(t("knowledge.chat.request.requirementFocus"));
    lines.push(t("knowledge.chat.request.requirementProposal"));
    lines.push(t("knowledge.chat.request.requirementStructure"));
    lines.push(t("knowledge.chat.request.userRequestHeader"));
    lines.push(input);

    return {
      text: lines.join("\n"),
      displayText: input,
    };
  },
});

function typeLabel(type: KnowledgeDocumentType) {
  return t(`knowledge.type.${type}`);
}

function scopeLabel(document: KnowledgeDocument) {
  return document.storageSource === "app"
    ? t("knowledge.scope.user")
    : t("knowledge.scope.project");
}

function editModeLabel(mode: KnowledgeEditMode) {
  if (mode === "inherit_parent") return t("knowledge.meta.editMode.inheritParent");
  if (mode === "auto") return t("knowledge.meta.editMode.auto");
  if (mode === "proposal") return t("knowledge.meta.editMode.proposal");
  return t("knowledge.meta.editMode.readOnly");
}

function trimmedContext(value: string, limit: number) {
  if (!value) return "";
  if (value.length <= limit) return value;
  return t("knowledge.chat.request.truncated", value.slice(0, limit), limit);
}

function handleSelectAgent(agentId: string) {
  manualKnowledgeAgentId.value = agentId;
}
</script>

<template>
  <EmbeddedChatPane
    :messages="messages"
    :streaming-text="streamingText"
    :streaming-text-order="streamingTextOrder"
    :thinking-text="thinkingText"
    :thinking-order="thinkingOrder"
    :is-streaming="isStreaming"
    :is-thinking="isThinking"
    :thinking-duration="thinkingDuration"
    :active-tool-calls="activeToolCalls"
    :pending-question="pendingQuestion"
    :pending-tool-confirms="pendingToolConfirms"
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
    @send="send"
    @cancel="cancel"
    @clear="resetSession"
    @answer-question="answerQuestion"
    @answer-tool-confirm="answerToolConfirm"
    @answer-all-tool-confirms="answerAllToolConfirms"
    @apply-knowledge-proposal="applyKnowledgeProposal"
    @ignore-knowledge-proposal="ignoreKnowledgeProposal"
  >
    <template #composer-start>
      <AgentSelector
        :agents="agentStore.agents"
        :selected-id="knowledgeAgentId || ''"
        :disabled="isStreaming"
        @select="handleSelectAgent"
      />
    </template>
    <template #composer-actions>
      <ModelEffortSelector
        :models="modelStore.availableModels"
        :selected-id="modelStore.selectedModelId"
        :effort="modelStore.effort"
        :efforts="modelStore.availableEfforts"
        :effort-supported="modelStore.effortSupported"
        :disabled="isStreaming"
        @select-model="modelStore.selectModel"
        @select-effort="modelStore.selectEffort"
      />
    </template>
  </EmbeddedChatPane>
</template>
