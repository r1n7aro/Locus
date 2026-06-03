import type { ViewPackageDetail, ViewPackageFile } from "../../services/view";

function normalizeRelPath(value: string): string {
  const parts: string[] = [];
  for (const part of value.replace(/\\/g, "/").split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      parts.pop();
      continue;
    }
    parts.push(part);
  }
  return parts.join("/");
}

export function viewPackageRelPath(detail: ViewPackageDetail | null, relPath: string): string {
  const normalized = normalizeRelPath(relPath);
  if (!detail) return normalized;
  if (detail.files.some((file) => file.relPath === normalized)) return normalized;
  const viewRoot = normalizeRelPath(detail.summary.packageRelPath || detail.manifest.id);
  return viewRoot ? normalizeRelPath(`${viewRoot}/${normalized}`) : normalized;
}

function fileByPath(detail: ViewPackageDetail | null, relPath: string): ViewPackageFile | null {
  return detail?.files.find((file) => file.relPath === relPath) ?? null;
}

export function viewFileContent(detail: ViewPackageDetail | null, relPath: string): string {
  return fileByPath(detail, viewPackageRelPath(detail, relPath))?.content ?? "";
}

export function sanitizeCssForPreview(css: string): string {
  return css
    .replace(/@import\s+[^;]+;/gi, "")
    .replace(/url\s*\([^)]*\)/gi, "none");
}
