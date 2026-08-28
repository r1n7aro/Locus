import type { KnowledgeDocumentType } from "../types";
import { openKnowledgeMarkdownPreviewWindow } from "../services/knowledgeMarkdownPreviewWindow";
import { useUiStore } from "../stores/ui";
import { useDisplaySettings } from "./useDisplaySettings";
import { useWorkspaceContextStore } from "../stores/workspaceContext";

export function useKnowledgeDocumentOpen() {
  const uiStore = useUiStore();
  const { state: displaySettings } = useDisplaySettings();
  const workspaceContextStore = useWorkspaceContextStore();

  function openInKnowledge(docType: KnowledgeDocumentType, path: string) {
    uiStore.stageKnowledgeSelection({
      dashboard: docType,
      path,
    });
    uiStore.setTab("chat");
  }

  async function openDocument(docType: KnowledgeDocumentType, path: string) {
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (
      workspaceRef
      &&
      docType === "memory"
      && displaySettings.memoryFileOpenTarget === "window"
      && await openKnowledgeMarkdownPreviewWindow({ docType, path, workspaceRef })
    ) {
      return;
    }
    openInKnowledge(docType, path);
  }

  return {
    openDocument,
    openInKnowledge,
  };
}
