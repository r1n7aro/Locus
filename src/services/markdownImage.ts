import { ipcInvoke } from "./ipc";

export interface MarkdownImagePreview {
  url: string;
  mimeType: string;
  byteSize: number;
  displayPath: string;
}

export function resolveMarkdownImage(source: string): Promise<MarkdownImagePreview> {
  return ipcInvoke<MarkdownImagePreview>("resolve_markdown_image", { source });
}
