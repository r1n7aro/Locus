<script setup lang="ts">
import {
  computed,
  defineAsyncComponent,
  nextTick,
  onMounted,
  ref,
  watch,
  type Component,
} from "vue";
import { listen } from "@tauri-apps/api/event";
import { initTheme } from "./composables/useTheme";
import { initFonts } from "./composables/useDisplaySettings";
import { markStartupPhase } from "./services/startupPerf";
import {
  getCurrentTauriWindowLabel,
  hasTauriWindowRuntime,
  showCurrentTauriWindow,
} from "./services/tauriRuntime";
import {
  SUB_WINDOW_ASSIGN_EVENT,
  SUB_WINDOW_ENTRY_PATH,
  isSubWindowPoolLocation,
  markSubWindowPoolReady,
  type SubWindowAssignPayload,
} from "./services/subWindow";
import { isKnowledgeDownloadWindowLocation } from "./services/knowledgeDownloadWindow";
import { isKnowledgeLexicalProgressWindowLocation } from "./services/knowledgeLexicalProgressWindow";
import { isFeishuReferenceImportWindowLocation } from "./services/feishuReferenceImportWindow";
import { isUnityReferenceImportWindowLocation } from "./services/unityReferenceImportWindow";
import { isReferenceExternalImportWindowLocation } from "./services/referenceExternalImportWindow";
import { isCollabSearchWindowLocation } from "./services/collabSearchWindow";
import { isChatDiffReviewWindowLocation } from "./services/chatDiffReviewWindow";
import { isPlanViewWindowLocation } from "./services/planViewWindow";
import { isUnityValueEditorWindowLocation } from "./services/unityValueEditorWindow";
import { isExtraWorkdirsWindowLocation } from "./services/extraWorkdirsWindow";
import { isViewContentWindowLocation, isViewHostWindowLocation } from "./services/view";
import { isAgentGraphToolWindowLocation } from "./services/agentGraphTool";
import SubWindowLoading from "./components/SubWindowLoading.vue";

// Router for the lightweight window.html entry: resolves which standalone
// window component the current location asks for, adopts pool assignments,
// and reveals the (hidden-created) native window once the shell rendered.

function asyncWindowComponent(loader: () => Promise<Component>) {
  return defineAsyncComponent({
    loader,
    loadingComponent: SubWindowLoading,
    delay: 80,
  });
}

interface WindowKindEntry {
  kind: string;
  matches: () => boolean;
  component: Component;
  props?: Record<string, unknown>;
  /** The component manages window visibility itself (View host flows). */
  selfRevealing?: boolean;
}

const WINDOW_KINDS: WindowKindEntry[] = [
  {
    kind: "knowledge-download",
    matches: isKnowledgeDownloadWindowLocation,
    component: asyncWindowComponent(() => import("./components/KnowledgeDownloadProgressWindow.vue")),
  },
  {
    kind: "knowledge-lexical-progress",
    matches: isKnowledgeLexicalProgressWindowLocation,
    component: asyncWindowComponent(() => import("./components/KnowledgeLexicalProgressWindow.vue")),
  },
  {
    kind: "feishu-reference-import",
    matches: isFeishuReferenceImportWindowLocation,
    component: asyncWindowComponent(() => import("./components/FeishuReferenceImportProgressWindow.vue")),
  },
  {
    kind: "unity-reference-import",
    matches: isUnityReferenceImportWindowLocation,
    component: asyncWindowComponent(() => import("./components/UnityReferenceImportProgressWindow.vue")),
  },
  {
    kind: "reference-external-import",
    matches: isReferenceExternalImportWindowLocation,
    component: asyncWindowComponent(() => import("./components/ReferenceExternalImportWindow.vue")),
  },
  {
    kind: "collab-search",
    matches: isCollabSearchWindowLocation,
    component: asyncWindowComponent(() => import("./components/CollabSearchWindow.vue")),
  },
  {
    kind: "chat-diff-review",
    matches: isChatDiffReviewWindowLocation,
    component: asyncWindowComponent(() => import("./components/ChatDiffReviewWindow.vue")),
  },
  {
    kind: "plan-view",
    matches: isPlanViewWindowLocation,
    component: asyncWindowComponent(() => import("./components/PlanViewWindow.vue")),
  },
  {
    kind: "unity-value-editor",
    matches: isUnityValueEditorWindowLocation,
    component: asyncWindowComponent(() => import("./components/UnityValueEditorWindow.vue")),
  },
  {
    kind: "extra-workdirs",
    matches: isExtraWorkdirsWindowLocation,
    component: asyncWindowComponent(() => import("./components/ExtraWorkdirsConfigWindow.vue")),
  },
  {
    kind: "view-content",
    matches: isViewContentWindowLocation,
    component: asyncWindowComponent(() => import("./components/ViewHostWindow.vue")),
    props: { embedded: true },
    selfRevealing: true,
  },
  {
    kind: "view-host",
    matches: isViewHostWindowLocation,
    component: asyncWindowComponent(() => import("./components/ViewHostWindow.vue")),
    selfRevealing: true,
  },
  {
    kind: "agent-graph",
    matches: isAgentGraphToolWindowLocation,
    component: asyncWindowComponent(() => import("./components/AgentGraphToolWindow.vue")),
  },
];

initTheme("main");
initFonts();

const isPoolWindow = isSubWindowPoolLocation();
// Bumped when a pool assignment rewrites the location so the kind
// resolvers (which read window.location live) re-evaluate.
const locationVersion = ref(0);

const activeEntry = computed<WindowKindEntry | null>(() => {
  void locationVersion.value;
  if (isPoolWindow && locationVersion.value === 0) return null;
  return WINDOW_KINDS.find((entry) => entry.matches()) ?? null;
});

let revealRequested = false;
function revealSubWindow(reason: string) {
  if (revealRequested || !hasTauriWindowRuntime()) return;
  revealRequested = true;
  // setTimeout instead of requestAnimationFrame: the window is still
  // hidden here and hidden webviews may never fire rAF callbacks.
  void nextTick(() => {
    window.setTimeout(() => {
      markStartupPhase("sub_window_reveal", { reason });
      void showCurrentTauriWindow().catch((error) => {
        console.warn("[sub-window] failed to reveal window:", error);
      });
    }, 0);
  });
}

watch(activeEntry, (entry) => {
  if (!entry) return;
  markStartupPhase("sub_window_kind_resolved", { kind: entry.kind });
  if (!entry.selfRevealing) {
    revealSubWindow(`kind:${entry.kind}`);
  }
}, { immediate: true });

onMounted(async () => {
  markStartupPhase("window_app_mounted", {
    href: window.location.href,
    pool: isPoolWindow,
  });

  if (!isPoolWindow) {
    // Unknown-kind fallback: never leave a hidden window invisible.
    if (!activeEntry.value) {
      console.warn("[sub-window] no window kind matched:", window.location.href);
      revealSubWindow("unmatched-location");
    }
    return;
  }

  if (!hasTauriWindowRuntime()) return;
  await listen<SubWindowAssignPayload>(SUB_WINDOW_ASSIGN_EVENT, (event) => {
    const query = event.payload?.query ?? "";
    window.history.replaceState(null, "", `${SUB_WINDOW_ENTRY_PATH}?${query}`);
    locationVersion.value += 1;
  });
  const label = getCurrentTauriWindowLabel();
  if (!label) return;
  try {
    // Only after the assign listener is live; the Rust pool claims
    // windows strictly after this call, so assignments can't be missed.
    await markSubWindowPoolReady(label);
    markStartupPhase("sub_window_pool_ready", { label });
  } catch (error) {
    console.warn("[sub-window] failed to mark pool window ready:", error);
  }
});
</script>

<template>
  <component
    :is="activeEntry.component"
    v-if="activeEntry"
    v-bind="activeEntry.props"
  />
  <div v-else class="sub-window-idle" />
</template>

<style scoped>
.sub-window-idle {
  position: fixed;
  inset: 0;
}
</style>
