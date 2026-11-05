<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from "vue";
import { useAssetState } from "../../composables/useAssetState";
import type { WorkspaceRef } from "../../services/project";
import { t } from "../../i18n";
import { workspaceFileEntries, type WorkspaceFileTreeState } from "./workspaceFileTree";
import { useWorkspaceFileOrder } from "../../composables/useWorkspaceFileOrder";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import { useWorkspaceFileVisibility, workspacePathIsHidden } from "../../composables/useWorkspaceFileVisibility";
import { subscribeExplorerFileActions } from "../../services/workspaceExplorer";
import { workspaceRelativeEntryPath } from "./workspaceFileMove";
import { normalizeAppError } from "../../services/errors";

// Renderless adapter: keep file watching and paging scoped to each mounted checkout,
// while the workbench renders files and special nodes in one virtualized tree.
const props = withDefaults(defineProps<{ workingDir: string; workspaceRef: WorkspaceRef; active?: boolean }>(), {
  active: true,
});
const emit = defineEmits<{ change: [state: WorkspaceFileTreeState]; error: [message: string] }>();
const state = useAssetState({
  get workingDir() { return props.workingDir; },
  get workspaceRef() { return props.workspaceRef; },
  get active() { return props.active; },
  includeFilesInTree: true,
  watchWorkspaceFiles: true,
});
const ordering = useWorkspaceFileOrder(props, message => emit("error", message));
const visibility = useWorkspaceFileVisibility();
// The parent creates a fresh workspaceRef object on every render. Depend on its
// stable checkout ID so emitting tree rows cannot trigger a render feedback loop.
const checkoutId = computed(() => props.workspaceRef.checkoutId);
const hiddenPaths = computed(() => visibility.paths(checkoutId.value));
const isHidden = (path: string) => workspacePathIsHidden(path, hiddenPaths.value);
let disposed = false;
let release: (() => void) | undefined;
void subscribeExplorerFileActions(event => {
  const rootKey = (root: string) => root.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
  if (rootKey(event.root) !== rootKey(props.workingDir)) return;
  if (event.workspaceRef && event.workspaceRef.checkoutId !== props.workspaceRef.checkoutId) return;
  const source = workspaceRelativeEntryPath(props.workingDir, event.path);
  const target = event.newPath === null ? null : workspaceRelativeEntryPath(props.workingDir, event.newPath);
  if (!source || (event.newPath !== null && !target)) return;
  try { visibility.remap(props.workspaceRef.checkoutId, source, target); }
  catch (error) { emit("error", normalizeAppError(error).message); }
}).then(unsubscribe => { if (disposed) unsubscribe(); else release = unsubscribe; }).catch(() => {});
onBeforeUnmount(() => { disposed = true; release?.(); });
function findFolder(nodes: AssetExplorerNode[], directory: string): AssetExplorerNode | undefined {
  for (const node of nodes) {
    if (node.path === directory) return node;
    if (node.kind === "folder") {
      const found = findFolder(node.children, directory);
      if (found) return found;
    }
  }
}
async function siblings(directory: string) {
  const identity = JSON.stringify([props.workingDir, props.workspaceRef]);
  const node = findFolder(state.explorerTree.value, directory);
  if (!node || node.kind !== "folder") throw new Error("The folder is no longer available.");
  // A drag is uncommon: finish only this directory's paging before compacting its
  // overrides, so unseen siblings are never mistaken for deleted entries.
  while (node.loading || node.hasMore) {
    if (node.loading) await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => { stop(); reject(new Error("The folder is still loading.")); }, 10000);
      const stop = watch(() => node.loading, loading => { if (!loading) { clearTimeout(timer); stop(); resolve(); } });
    });
    if (identity !== JSON.stringify([props.workingDir, props.workspaceRef])) throw new Error("The workspace changed during the drag.");
    if (!node.hasMore) break;
    const offset = node.nextOffset;
    await state.loadMoreFolder(directory);
    if (node.nextOffset === offset && node.hasMore) throw new Error(state.error.value || "The folder could not be loaded.");
  }
  return workspaceFileEntries(node.children, () => false, t("asset.explorer.loadMore"), isHidden);
}
const entries = computed(() => workspaceFileEntries(state.explorerTree.value, state.isPathExpanded, t("asset.explorer.loadMore"), isHidden));
watch([entries, ordering.order], ([value, order]) => emit("change", { entries: value, toggle: state.togglePath,
  reveal: state.revealPath, loadMore: state.loadMoreFolder, refresh: state.applyExplorerFileChange, order, siblings, move: ordering.move }), { immediate: true });
watch(state.error, message => { if (message) emit("error", message); });
</script>

<template></template>
