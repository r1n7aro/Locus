import type { KnowledgeSourceConfig } from "../../types";

function normalizePath(value: string): string {
  return value
    .trim()
    .replace(/[\\/]+/g, "/")
    .replace(/\/+$/, "")
    .toLowerCase();
}

function isAbsolutePath(value: string): boolean {
  return /^[a-z]:[\\/]/i.test(value) || value.startsWith("\\\\") || value.startsWith("/");
}

function resolveRootPath(rootPath: string, workingDir?: string): string {
  const trimmed = rootPath.trim();
  if (!trimmed) return "";
  if (isAbsolutePath(trimmed)) return normalizePath(trimmed);
  if (!workingDir?.trim()) return normalizePath(trimmed);
  return normalizePath(`${workingDir}/${trimmed}`);
}

function isWithinPath(basePath: string, targetPath: string): boolean {
  if (!basePath || !targetPath) return false;
  return targetPath === basePath || targetPath.startsWith(`${basePath}/`);
}

export function isExternalKnowledgeSource(source: KnowledgeSourceConfig, workingDir?: string): boolean {
  if (source.type === "feishu") return true;
  const resolvedRoot = resolveRootPath(source.rootPath, workingDir);
  const normalizedWorkingDir = normalizePath(workingDir ?? "");
  if (!resolvedRoot || !normalizedWorkingDir) return false;
  return !isWithinPath(normalizedWorkingDir, resolvedRoot);
}

export function filterExternalKnowledgeSources(
  sources: KnowledgeSourceConfig[],
  workingDir?: string,
): KnowledgeSourceConfig[] {
  return sources.filter((source) => isExternalKnowledgeSource(source, workingDir));
}
