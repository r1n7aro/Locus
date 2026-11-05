import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";

export interface MarkdownImagePreview {
  url: string;
  mimeType: string;
  byteSize: number;
  displayPath: string;
}

export function resolveMarkdownImage(workspaceRef: WorkspaceRef, source: string, contentPath?: string): Promise<MarkdownImagePreview> {
  return ipcInvoke<MarkdownImagePreview>("resolve_markdown_image", { workspaceRef, source, ...(contentPath ? { contentPath } : {}) });
}

export interface ImportedMarkdownImage { path: string; relativePath: string }

/** Project settings that choose where pasted Markdown images are stored. */
export const MARKDOWN_IMAGE_DIRECTORY_KEY = "project.markdownImages.directory";
export const MARKDOWN_IMAGE_RULES_KEY = "project.markdownImages.rules";
/** Suggested default folder (where older builds always stored images). */
export const DEFAULT_MARKDOWN_IMAGE_DIRECTORY = "Locus/attachments/images";
/** `markdown_import_image` error while the project has no folder for the document. */
export const MARKDOWN_IMAGE_DIRECTORY_UNSET = "markdown.image_directory_unset";

export interface MarkdownImageTarget {
  /** Workspace root with `/` separators. */
  root: string;
  /** Workspace-relative folder of the document; `""` at the root. */
  documentFolder: string;
  /** Folder receiving pasted images; `null` until the project chooses one. */
  folder: string | null;
}

export function markdownImageTarget(workspaceRef: WorkspaceRef, contentPath: string): Promise<MarkdownImageTarget> {
  return ipcInvoke("markdown_image_target", { workspaceRef, contentPath });
}

/**
 * Workspace-relative form of a picked folder, or `null` when it is not inside
 * `root`. The backend normalizes and validates the stored value again.
 */
export function workspaceRelativeFolder(root: string, folder: string): string | null {
  const normalize = (value: string) => value.replace(/\\/g, "/").replace(/\/+$/, "");
  const base = normalize(root);
  const target = normalize(folder);
  // Windows paths compare case-insensitively.
  const caseless = /^[a-z]:\//i.test(base);
  const same = (left: string, right: string) => caseless ? left.toLowerCase() === right.toLowerCase() : left === right;
  if (!base || same(target, base)) return null;
  const prefix = target.slice(0, base.length + 1);
  return same(prefix, `${base}/`) ? target.slice(base.length + 1) : null;
}

export function relativeMarkdownResource(workspaceRef: WorkspaceRef, contentPath: string, targetPath: string): Promise<string> {
  return ipcInvoke("markdown_relative_path", { workspaceRef, contentPath, targetPath });
}
export async function importMarkdownImage(workspaceRef: WorkspaceRef, contentPath: string, source: File | string): Promise<ImportedMarkdownImage> {
  if (typeof source === "string") return ipcInvoke("markdown_import_image", { workspaceRef, contentPath, name: source, sourcePath: source });
  if (source.size > 20 * 1024 * 1024) throw new Error("图片不能超过 20 MiB");
  const data = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(source);
  });
  return ipcInvoke("markdown_import_image", { workspaceRef, contentPath, name: source.name, data });
}

export function rewriteMarkdownImageLinks(workspaceRef: WorkspaceRef, contentPath: string, text: string, oldPath: string, newPath: string): Promise<string> {
  return ipcInvoke("markdown_rewrite_image_links", { workspaceRef, contentPath, text, oldPath, newPath });
}
