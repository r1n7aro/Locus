import { ref, watch } from "vue";

const STORAGE_KEY = "locus-hide-meta-files";
const META_SUFFIX = ".meta";
const LOCUS_KNOWLEDGE_PREFIX = "Locus/knowledge/";

type MetaPathEntry = string | {
  path: string;
  oldPath?: string | null;
  primaryExistsInWorkspace?: boolean | null;
  primaryIsDirectoryInWorkspace?: boolean | null;
};

function normalizeComparablePath(path: string): string {
  return path.replace(/\\/g, "/");
}

function readInitialHideMeta(): boolean {
  try {
    if (typeof localStorage === "undefined") return true;
    return localStorage.getItem(STORAGE_KEY) !== "false";
  } catch {
    return true;
  }
}

const hideMeta = ref(readInitialHideMeta());

watch(hideMeta, (v) => {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(STORAGE_KEY, String(v));
  } catch {
    // Ignore persistence failures and keep the in-memory toggle working.
  }
});

export function useHideMeta() {
  return { hideMeta };
}

export function isMetaFile(path: string): boolean {
  return path.endsWith(META_SUFFIX);
}

export function primaryPathForMeta(path: string): string | null {
  if (!isMetaFile(path)) return null;
  return path.slice(0, -META_SUFFIX.length);
}

function normalizeMetaPathEntry(entry: MetaPathEntry) {
  if (typeof entry === "string") {
    return {
      path: entry,
      normalizedPath: normalizeComparablePath(entry),
      oldPath: null,
      normalizedOldPath: null,
      primaryExistsInWorkspace: null,
      primaryIsDirectoryInWorkspace: null,
    };
  }

  return {
    path: entry.path,
    normalizedPath: normalizeComparablePath(entry.path),
    oldPath: entry.oldPath ?? null,
    normalizedOldPath: entry.oldPath ? normalizeComparablePath(entry.oldPath) : null,
    primaryExistsInWorkspace: entry.primaryExistsInWorkspace ?? null,
    primaryIsDirectoryInWorkspace: entry.primaryIsDirectoryInWorkspace ?? null,
  };
}

function isLegacyLocusKnowledgeMeta(path: string): boolean {
  const normalized = normalizeComparablePath(path);
  return normalized.startsWith(LOCUS_KNOWLEDGE_PREFIX) && normalized.endsWith(META_SUFFIX);
}

function hasListedDescendant(
  allPaths: Set<string>,
  scopePaths: Iterable<string | null>,
  excludedPaths: Set<string>,
): boolean {
  for (const scopePath of scopePaths) {
    if (!scopePath) continue;
    const prefix = scopePath.endsWith("/") ? scopePath : `${scopePath}/`;
    for (const candidatePath of allPaths) {
      if (excludedPaths.has(candidatePath)) continue;
      if (candidatePath.startsWith(prefix)) return true;
    }
  }
  return false;
}

export function partitionMetaPaths(entries: Iterable<MetaPathEntry>) {
  const allPaths = new Set<string>();
  const entryInfo = new Map<string, ReturnType<typeof normalizeMetaPathEntry>>();
  const hideableMetaPaths = new Set<string>();
  const orphanMetaPaths = new Set<string>();

  for (const rawEntry of entries) {
    const entry = normalizeMetaPathEntry(rawEntry);
    allPaths.add(entry.normalizedPath);
    entryInfo.set(entry.normalizedPath, entry);
  }

  for (const [path, entry] of entryInfo) {
    if (isLegacyLocusKnowledgeMeta(path)) continue;
    const primaryPath = primaryPathForMeta(path);
    if (!primaryPath) continue;
    if (allPaths.has(primaryPath)) {
      hideableMetaPaths.add(entry.path);
    } else {
      // Git working-tree lists rarely contain directory nodes, so folder sidecars
      // like `Assets/UI.meta` need a filesystem-backed primary check.
      const hasWorkspacePrimary = entry?.primaryExistsInWorkspace === true;
      const hasWorkspaceDirectory = entry?.primaryIsDirectoryInWorkspace === true;
      const oldPrimaryPath = entry?.normalizedOldPath
        ? primaryPathForMeta(entry.normalizedOldPath)
        : null;
      const hasListedContext = hasListedDescendant(
        allPaths,
        [primaryPath, oldPrimaryPath],
        new Set([
          path,
          entry?.normalizedOldPath ?? "",
        ]),
      );
      const isRenamedMeta = !!entry?.normalizedOldPath && entry.normalizedOldPath !== path;
      if (hasWorkspacePrimary || hasWorkspaceDirectory || hasListedContext || isRenamedMeta) {
        continue;
      }
      orphanMetaPaths.add(entry.path);
    }
  }

  return {
    hideableMetaPaths,
    orphanMetaPaths,
  };
}

/**
 * When `.meta` entries are hidden in the UI, keep sidecar operations aligned
 * with the visible primary file if the companion exists in the same source list.
 */
export function withMetaCompanionPaths(
  paths: Iterable<string>,
  sourcePaths: Iterable<string>,
  includeMetaCompanions: boolean,
): string[] {
  const uniquePaths = new Set(paths);
  if (!includeMetaCompanions) return [...uniquePaths];
  const availablePaths = new Set(sourcePaths);
  for (const path of [...uniquePaths]) {
    if (isMetaFile(path)) continue;
    const metaPath = `${path}${META_SUFFIX}`;
    if (availablePaths.has(metaPath)) {
      uniquePaths.add(metaPath);
    }
  }
  return [...uniquePaths];
}

const UNITY_SERIALIZED_EXTS = new Set([
  ".unity", ".prefab", ".asset", ".mat", ".anim", ".controller",
  ".overrideController", ".physicMaterial", ".physicsMaterial2D",
  ".fontsettings", ".guiskin", ".mask", ".flare", ".renderTexture",
  ".lighting", ".preset", ".signal", ".playable", ".terrainlayer",
  ".brush", ".meta",
]);

const BINARY_EXTS = new Set([
  ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tga", ".psd", ".tif", ".tiff",
  ".exr", ".hdr", ".webp", ".ico", ".svg",
  ".fbx", ".obj", ".blend", ".dae", ".3ds", ".max", ".mb", ".ma",
  ".wav", ".mp3", ".ogg", ".aif", ".aiff", ".flac", ".wma",
  ".mp4", ".avi", ".mov", ".wmv", ".webm",
  ".dll", ".so", ".dylib", ".exe", ".a", ".lib",
  ".ttf", ".otf", ".woff", ".woff2",
  ".zip", ".rar", ".7z", ".gz", ".tar",
  ".pdf", ".doc", ".docx", ".xls", ".xlsx",
]);

/** Returns true if the file is suitable for opening in a text editor. */
export function canOpenInEditor(path: string): boolean {
  const dot = path.lastIndexOf(".");
  if (dot < 0) return true;
  const ext = path.substring(dot).toLowerCase();
  return !UNITY_SERIALIZED_EXTS.has(ext) && !BINARY_EXTS.has(ext);
}
