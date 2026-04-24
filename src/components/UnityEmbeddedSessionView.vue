<script setup lang="ts">
import { t } from "../i18n";
import ChatWorkspaceView from "./ChatWorkspaceView.vue";
import TopBannerHost from "./TopBannerHost.vue";

withDefaults(defineProps<{
  bootstrapped?: boolean;
  bootstrapError?: string | null;
}>(), {
  bootstrapped: false,
  bootstrapError: null,
});
</script>

<template>
  <main class="unity-session-view">
    <TopBannerHost />

    <div v-if="bootstrapError" class="unity-session-state is-error">
      {{ bootstrapError }}
    </div>
    <div v-else-if="!bootstrapped" class="unity-session-state">
      {{ t("common.loading") }}
    </div>
    <ChatWorkspaceView
      v-else
      class="unity-session-workspace"
      active
      layout-mode="auto"
    />
  </main>
</template>

<style scoped>
.unity-session-view {
  display: flex;
  flex-direction: column;
  width: 100vw;
  height: 100vh;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: var(--bg-color);
  color: var(--text-color);
}

.unity-session-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  background: var(--panel-bg);
  color: var(--text-secondary);
  font-size: 13px;
  line-height: 1.5;
  text-align: center;
}

.unity-session-state.is-error {
  color: var(--status-danger-fg);
}

.unity-session-workspace {
  flex: 1;
  min-width: 0;
  min-height: 0;
}

:deep(.top-banner-host) {
  top: 10px;
}
</style>
