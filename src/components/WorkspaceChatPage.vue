<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { t } from "../i18n";
import { useAppBootstrap } from "../composables/useAppBootstrap";
import { normalizeAppError } from "../services/errors";
import { useAgentStore } from "../stores/agent";
import { useChatStore } from "../stores/chat";
import { useUiStore } from "../stores/ui";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import ChatWorkspaceView from "./ChatWorkspaceView.vue";

const agentStore = useAgentStore();
const chatStore = useChatStore();
const uiStore = useUiStore();
const workspaceContextStore = useWorkspaceContextStore();
const ready = ref(false);
const error = ref("");
const storageScope = computed(() => (
  `workspace-page-${workspaceContextStore.focusedWorkspaceRef?.checkoutId ?? ""}`
));
const {
  bootstrapCritical,
  registerListeners,
  cleanup,
} = useAppBootstrap({
  syncActiveSessionSelection: false,
  handleExternalScriptOpen: false,
});

onMounted(async () => {
  try {
    uiStore.setTab("chat");
    await bootstrapCritical();
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!workspaceRef) throw new Error(t("app.tab.windowUnavailable"));
    await agentStore.loadWorkspaceAgents(workspaceRef);
    await chatStore.refreshSessions();
    await registerListeners();
    ready.value = true;
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  }
});

onUnmounted(() => cleanup());
</script>

<template>
  <div v-if="error" class="workspace-chat-page-state is-error">{{ error }}</div>
  <div v-else-if="!ready" class="workspace-chat-page-state">{{ t("common.loading") }}</div>
  <ChatWorkspaceView
    v-else
    active
    layout-mode="auto"
    :session-panel-storage-scope="storageScope"
    :persist-session-selection="true"
  />
</template>

<style scoped>
.workspace-chat-page-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  min-height: 0;
  color: var(--text-secondary);
  font-size: 13px;
}

.workspace-chat-page-state.is-error {
  color: var(--status-danger-fg);
}
</style>
