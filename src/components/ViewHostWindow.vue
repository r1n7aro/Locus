<script setup lang="ts">
import { onMounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { viewHostIdFromLocation, viewWorkspaceRefFromLocation, isExactViewWorkspaceBinding } from "../services/view";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { normalizeAppError } from "../services/errors";
import ViewRuntimeHost from "./view/ViewRuntimeHost.vue";
defineProps<{ embedded?: boolean }>();
const viewId = viewHostIdFromLocation();
const workspaceRef = viewWorkspaceRefFromLocation();
const workspace = useWorkspaceContextStore();
const ready = ref(false);
const error = ref("");
const label = getCurrentWindow().label;
onMounted(async () => {
  try {
    if (!workspaceRef) throw new Error("View window is missing its checkout binding.");
    await workspace.initialize(label, "main");
    const context = await workspace.focusCheckout(workspaceRef.checkoutId);
    if (!context || !isExactViewWorkspaceBinding(workspaceRef, { checkoutId: context.focusedCheckoutId, workspaceGeneration: context.workspaceGeneration, materializationEpoch: context.materializationEpoch })) throw new Error("View workspace binding is stale.");
    ready.value = true;
  } catch (failure) { error.value = normalizeAppError(failure).message; }
});
</script>
<template>
  <ViewRuntimeHost v-if="ready && workspaceRef" :view-id="viewId" :workspace-ref="workspaceRef" :instance-id="label + ':' + viewId" :window-label="label" />
  <div v-else-if="error" class="view-host-error" role="alert">{{ error }}</div>
</template>
<style scoped>
.view-host-error { padding: 12px; color: var(--status-error-fg); font-size: 12px; }
</style>
