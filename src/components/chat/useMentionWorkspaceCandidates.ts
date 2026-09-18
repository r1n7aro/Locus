import { computed, type ComputedRef } from "vue";
import { workspaceMaterializationMatches, type WorkspaceRef } from "../../services/project";
import type { LocusFileDropRef } from "../../services/unity";
import { useWorkbenchStore } from "../../stores/workbench";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import type { MentionSearchRankable } from "./mentionSearchRanking";

export interface MentionSearchResult extends MentionSearchRankable {
  isDir: boolean;
  localFile?: LocusFileDropRef;
}

function normalizePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function relativePath(path: string, root: string): string | null {
  return path.toLowerCase().startsWith(`${root.toLowerCase()}/`)
    ? path.slice(root.length + 1) : null;
}

export function useMentionWorkspaceCandidates(
  workspaceRef: ComputedRef<WorkspaceRef | null>,
  workspaceRoot: ComputedRef<string>,
) {
  const workbench = useWorkbenchStore();
  const context = useWorkspaceContextStore();
  const explorer = useWorkspaceExplorerStore();

  // Only consume data already loaded by the workbench. This never opens tabs,
  // loads a tree, or issues IPC in response to a keystroke.
  return computed<MentionSearchResult[]>(() => {
    const scope = workspaceRef.value;
    if (!scope) return [];
    const checkout = context.checkoutsById[scope.checkoutId];
    if (!checkout || checkout.available === false) return [];
    const root = normalizePath(workspaceRoot.value || checkout.root);
    if (!root) return [];
    const projectId = checkout.projectId;
    const documents = explorer.resources[projectId]?.knowledge ?? [];
    const scopedDocuments = documents.filter((document) => (
      (document.storageSource ?? "project") === "project"
      && document.availableCheckoutIds.includes(scope.checkoutId)
    ));
    const documentsById = new Map(scopedDocuments.map((document) => [document.id, document]));
    const otherRoots = Object.values(context.checkoutsById)
      .filter((item) => item.checkoutId !== scope.checkoutId)
      .map((item) => normalizePath(item.root)).filter(Boolean);
    const results: MentionSearchResult[] = [];

    function add(path: string, name: string, isDir: boolean,
      source: "tab" | "tree", entryKind: MentionSearchResult["entryKind"] = "asset") {
      path = normalizePath(path);
      if (!path) return;
      const absolute = /^(?:[a-z]:\/|\/)/i.test(path);
      let localFile: LocusFileDropRef | undefined;
      if (absolute) {
        // A physical mount in another checkout must not become a relative ref here.
        if (otherRoots.some((other) => path.toLowerCase() === other.toLowerCase()
          || relativePath(path, other) !== null)) return;
        const relative = relativePath(path, root);
        if (relative !== null) path = relative;
        else localFile = { path, name, isDir, source: "local" };
      }
      if (!localFile && !isDir) {
        const knowledge = path.match(/^Locus\/knowledge\/((?:design|plan|memory|skill|reference)\/.+\.md)$/i);
        if (knowledge) {
          path = knowledge[1]!;
          entryKind = "knowledge";
        }
      }
      results.push({
        relPath: path, name, isDir, entryKind, source, localFile, matchScore: 0,
        parentPath: path.slice(0, Math.max(0, path.lastIndexOf("/"))),
      });
    }

    const window = workbench.windows[context.windowId];
    if (window) {
      const groups = Object.values(window.groups).sort((a, b) => (
        Number(b.paneId === window.focusedPaneId) - Number(a.paneId === window.focusedPaneId)
      ));
      for (const group of groups) {
        const tabs = [...group.tabs].sort((a, b) => (
          Number(b.editorId === group.activeEditorId) - Number(a.editorId === group.activeEditorId)
        ));
        for (const tab of tabs) {
          if (tab.availability !== "available" || tab.resource.projectId !== projectId) continue;
          const binding = tab.checkoutBinding;
          if (binding && (binding.checkoutId !== scope.checkoutId
            || (binding.expectedGeneration != null && scope.expectedGeneration != null
              && binding.expectedGeneration !== scope.expectedGeneration)
            || !workspaceMaterializationMatches(scope.expectedMaterializationEpoch, binding.expectedMaterializationEpoch))) continue;
          const resource = tab.resource;
          switch (resource.kind) {
            case "asset":
            case "workspaceFile":
              if (binding) add(resource.path, tab.title, false, "tab");
              break;
            case "sceneObject":
              if (binding) add(`${resource.scenePath}/${resource.objectPath}`, tab.title, false, "tab", "sceneObject");
              break;
            case "knowledge": {
              const document = documentsById.get(resource.documentId);
              if (document) add(`${document.type}/${document.path}`, document.title || tab.title, false, "tab", "knowledge");
              break;
            }
            case "localFile":
            case "localDirectory":
            case "folder":
              if (tab.sourcePath) add(tab.sourcePath, tab.title, resource.kind !== "localFile", "tab");
              break;
          }
        }
      }
    }

    const snapshot = explorer.snapshots[projectId];
    const nodes = snapshot?.nodes ?? [];
    const nodesById = new Map(nodes.map((node) => [node.nodeId, node]));
    function isVisible(node: typeof nodes[number]): boolean {
      const seen = new Set<string>();
      let current: typeof node | undefined = node;
      while (current) {
        if (current.hidden || seen.has(current.nodeId)) return false;
        seen.add(current.nodeId);
        current = current.parentNodeId ? nodesById.get(current.parentNodeId) : undefined;
      }
      return true;
    }
    for (const node of nodes) {
      if (!isVisible(node)) continue;
      if (node.resourceKind === "knowledge" && node.resourceId) {
        const document = documentsById.get(node.resourceId);
        if (document) add(`${document.type}/${document.path}`, document.title || document.path, false, "tree", "knowledge");
      }
      if (!node.sourcePath) continue;
      const path = normalizePath(node.sourcePath);
      add(path, node.folderName || path.split("/").pop() || path, node.nodeKind === "folder", "tree");
      const listing = explorer.mountListing(projectId, node.nodeId);
      for (const entry of listing?.entries ?? []) {
        add(entry.absolutePath, entry.name, entry.isDir, "tree");
      }
    }
    return results;
  });
}
