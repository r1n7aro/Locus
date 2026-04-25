import { t } from "../../i18n";
import type { KnowledgeToolConfirmPreview, PendingToolConfirm } from "../../types";

export function editorStatusLabelForToolConfirm(status: string): string {
  const key = `chat.toolConfirm.unityStatus.status.${status}`;
  const label = t(key);
  return label === key ? status : label;
}

export function titleForUnityEditorStatusChange(requestedStatus: string): string {
  const key = `chat.toolConfirm.unityStatus.title.${requestedStatus}`;
  const label = t(key);
  return label === key ? t("chat.toolConfirm.unityStatus.title") : label;
}

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
  if (toolConfirm.display.kind === "unityEditorStatusChange") {
    return titleForUnityEditorStatusChange(toolConfirm.display.requestedStatus);
  }
  return toolConfirm.display.toolName;
}

export function subtitleForPendingToolConfirm(toolConfirm: PendingToolConfirm): string {
  if (toolConfirm.display.kind === "knowledge") {
    return toolConfirm.display.newPath?.trim() || toolConfirm.display.path;
  }
  if (toolConfirm.display.kind === "unityEditorStatusChange") {
    return t(
      "chat.toolConfirm.unityStatus.subtitle",
      editorStatusLabelForToolConfirm(toolConfirm.display.currentStatus),
      editorStatusLabelForToolConfirm(toolConfirm.display.requestedStatus),
    );
  }
  return toolConfirm.display.toolName;
}
