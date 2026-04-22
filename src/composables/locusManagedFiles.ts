export interface LocusManagedFileLike {
  path: string;
  oldPath?: string;
}

const LOCUS_PATH_PREFIXES = [
  "Locus",
  "Library/Locus",
  "Assets/Locus",
  "Assets/Plugins/Locus",
] as const;

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

export function countLocusManagedFiles(files: Iterable<LocusManagedFileLike>): number {
  let count = 0;
  for (const file of files) {
    if (isLocusManagedFile(file)) count += 1;
  }
  return count;
}
