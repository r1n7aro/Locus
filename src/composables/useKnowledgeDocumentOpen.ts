import type { KnowledgeDocumentType } from "../types";
import { openKnowledgeMarkdownPreviewWindow } from "../services/knowledgeMarkdownPreviewWindow";
import { useUiStore } from "../stores/ui";
import { useDisplaySettings } from "./useDisplaySettings";

export function useKnowledgeDocumentOpen() {
  const uiStore = useUiStore();
  const { state: displaySettings } = useDisplaySettings();

  function openInKnowledge(docType: KnowledgeDocumentType, path: string) {
    uiStore.stageKnowledgeSelection({
      dashboard: docType,
      path,
    });
    uiStore.setTab("knowledge");
  }

  async function openDocument(docType: KnowledgeDocumentType, path: string) {
    if (
      docType === "memory"
      && displaySettings.memoryFileOpenTarget === "window"
      && await openKnowledgeMarkdownPreviewWindow({ docType, path })
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
