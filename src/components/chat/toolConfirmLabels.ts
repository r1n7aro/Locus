import { t } from "../../i18n";
import type { KnowledgeToolConfirmPreview, PendingToolConfirm } from "../../types";

export function titleForKnowledgeToolConfirm(preview: KnowledgeToolConfirmPreview): string {
  const docTypeTitle = t(`chat.toolConfirm.knowledge.docType.${preview.docType}`);
  const key = preview.targetKind === "directory"
    ? `chat.toolConfirm.knowledge.title.${preview.operation}Directory`
    : `chat.toolConfirm.knowledge.title.${preview.operation}Document`;
  return t(key, docTypeTitle);
}

export function titleForPendingToolConfirm(toolConfirm: PendingToolConfirm): string {
  if (toolConfirm.display.kind === "knowledge") {
    return titleForKnowledgeToolConfirm(toolConfirm.display);
  }
  return toolConfirm.display.toolName;
}

export function subtitleForPendingToolConfirm(toolConfirm: PendingToolConfirm): string {
  if (toolConfirm.display.kind === "knowledge") {
    return toolConfirm.display.newPath?.trim() || toolConfirm.display.path;
  }
  return toolConfirm.display.toolName;
}
