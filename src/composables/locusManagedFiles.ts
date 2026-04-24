export interface LocusManagedFileLike {
  path: string;
  oldPath?: string;
}

export type LocusManagedTagKind = "locus" | "design" | "memory" | "skill" | "reference";

const LOCUS_PATH_PREFIXES = [
  "Locus",
  "Library/Locus",
  "Assets/Locus",
  "Assets/Plugins/Locus",
  "Packages/com.farlocus.locus",
] as const;

const KNOWLEDGE_TYPE_TAGS: Record<string, LocusManagedTagKind> = {
  design: "design",
  memory: "memory",
  skill: "skill",
  reference: "reference",
};

function normalizeRepoPath(path: string | undefined | null): string {
  return (path ?? "")
    .split("\\")
    .join("/")
    .replace(/^\.\//, "")
    .trim();
}

export function isLocusManagedPath(path: string): boolean {
  const normalized = normalizeRepoPath(path);
  if (!normalized) return false;
  if (LOCUS_PATH_PREFIXES.some((prefix) =>
    normalized === prefix
    || normalized.startsWith(`${prefix}/`)
    || normalized.startsWith(`${prefix}.`),
  )) {
    return true;
  }
  return false;
}

export function isLocusManagedFile(file: LocusManagedFileLike): boolean {
  return isLocusManagedPath(file.path) || isLocusManagedPath(file.oldPath ?? "");
}

function locusKnowledgeTagForPath(path: string | undefined | null): LocusManagedTagKind | null {
  const normalized = normalizeRepoPath(path);
  if (!normalized) return null;
  const parts = normalized.split("/");
  if (parts[0] !== "Locus") return null;

  if (parts[1] === "knowledge") {
    const typeSegment = (parts[2] ?? "").replace(/\.meta$/i, "").toLowerCase();
    return KNOWLEDGE_TYPE_TAGS[typeSegment] ?? null;
  }

  if ((parts[1] ?? "").toLowerCase().replace(/\.meta$/i, "") === "memory") {
    return "memory";
  }

  return null;
}

export function getLocusManagedTagKindForPath(path: string): LocusManagedTagKind | null {
  return locusKnowledgeTagForPath(path) ?? (isLocusManagedPath(path) ? "locus" : null);
}

export function getLocusManagedTagKind(file: LocusManagedFileLike): LocusManagedTagKind | null {
  return locusKnowledgeTagForPath(file.path)
    ?? locusKnowledgeTagForPath(file.oldPath)
    ?? (isLocusManagedFile(file) ? "locus" : null);
}

export function countLocusManagedFiles(files: Iterable<LocusManagedFileLike>): number {
  let count = 0;
  for (const file of files) {
    if (isLocusManagedFile(file)) count += 1;
  }
  return count;
}
