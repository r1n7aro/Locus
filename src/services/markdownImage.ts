import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";

export interface MarkdownImagePreview {
  url: string;
  mimeType: string;
  byteSize: number;
  displayPath: string;
}

export function resolveMarkdownImage(workspaceRef: WorkspaceRef, source: string): Promise<MarkdownImagePreview> {
  return ipcInvoke<MarkdownImagePreview>("resolve_markdown_image", { workspaceRef, source });
}
