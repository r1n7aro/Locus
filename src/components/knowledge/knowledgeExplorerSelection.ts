export interface ResolveKnowledgeExplorerSelectionInput {
  visiblePaths: string[];
  selectedPaths: Set<string>;
  lastAnchorPath: string | null;
  clickedPath: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  seedPath?: string | null;
}

export interface ResolveKnowledgeExplorerSelectionResult {
  nextSelectedPaths: Set<string>;
  nextLastAnchorPath: string | null;
  shouldHandleAsPlainClick: boolean;
}

export interface ResolveKnowledgeContextSelectionInput {
  visiblePaths: string[];
  selectedPaths: Set<string>;
  targetPath: string;
}

export interface KnowledgeDeleteTarget {
  kind: "folder" | "document";
  path: string;
}

function isDescendantPath(path: string, ancestor: string): boolean {
  if (!ancestor) return false;
  return path === ancestor || path.startsWith(`${ancestor}/`);
}

export function resolveKnowledgeExplorerSelection(
  input: ResolveKnowledgeExplorerSelectionInput,
): ResolveKnowledgeExplorerSelectionResult {
  const {
    visiblePaths,
    selectedPaths,
    lastAnchorPath,
    clickedPath,
    shiftKey,
    ctrlKey,
    metaKey,
    seedPath = null,
  } = input;

  const clickedIndex = visiblePaths.indexOf(clickedPath);
  if (clickedIndex < 0) {
    return {
      nextSelectedPaths: new Set(selectedPaths),
      nextLastAnchorPath: lastAnchorPath,
      shouldHandleAsPlainClick: false,
    };
  }

  if (shiftKey) {
    const anchorPath = lastAnchorPath && visiblePaths.includes(lastAnchorPath)
      ? lastAnchorPath
      : seedPath && visiblePaths.includes(seedPath)
        ? seedPath
        : null;
    const anchorIndex = anchorPath ? visiblePaths.indexOf(anchorPath) : -1;
    if (anchorIndex >= 0) {
      const [start, end] = anchorIndex <= clickedIndex
        ? [anchorIndex, clickedIndex]
        : [clickedIndex, anchorIndex];
      return {
        nextSelectedPaths: new Set(visiblePaths.slice(start, end + 1)),
        nextLastAnchorPath: anchorPath,
        shouldHandleAsPlainClick: false,
      };
    }
    return {
      nextSelectedPaths: new Set([clickedPath]),
      nextLastAnchorPath: clickedPath,
      shouldHandleAsPlainClick: false,
    };
  }

  if (ctrlKey || metaKey) {
    const next = new Set(selectedPaths);
    if (next.has(clickedPath)) {
      next.delete(clickedPath);
    } else {
      if (next.size === 0 && seedPath && seedPath !== clickedPath && visiblePaths.includes(seedPath)) {
        next.add(seedPath);
      }
      next.add(clickedPath);
    }
    return {
      nextSelectedPaths: next,
      nextLastAnchorPath: clickedPath,
      shouldHandleAsPlainClick: false,
    };
  }

  return {
    nextSelectedPaths: new Set(),
    nextLastAnchorPath: clickedPath,
    shouldHandleAsPlainClick: true,
  };
}

export function resolveKnowledgeContextSelection(
  input: ResolveKnowledgeContextSelectionInput,
): string[] {
  const { visiblePaths, selectedPaths, targetPath } = input;
  if (selectedPaths.size > 1 && selectedPaths.has(targetPath)) {
    return visiblePaths.filter((path) => selectedPaths.has(path));
  }
  return [targetPath];
}

export interface KnowledgeDragNodeRef {
  path: string;
}

/**
 * Reduce a multi-selection drag set to its outermost nodes. A node whose tree
 * path sits inside another dragged node already moves with that ancestor, so
 * moving it separately would double-move it (and fail once the ancestor has
 * landed in the target directory).
 */
export function pruneKnowledgeDragNodes<T extends KnowledgeDragNodeRef>(
  nodes: T[],
): T[] {
  const paths = new Set(nodes.map((node) => node.path));
  const seen = new Set<string>();
  return nodes.filter((node) => {
    if (seen.has(node.path)) return false;
    seen.add(node.path);
    let current = node.path;
    while (true) {
      const slash = current.lastIndexOf("/");
      if (slash < 0) return true;
      current = current.slice(0, slash);
      if (paths.has(current)) return false;
    }
  });
}

export function pruneKnowledgeDeleteTargets(
  targets: KnowledgeDeleteTarget[],
): KnowledgeDeleteTarget[] {
  const folderMap = new Map<string, KnowledgeDeleteTarget>();
  const documentMap = new Map<string, KnowledgeDeleteTarget>();

  for (const target of targets) {
    if (target.kind === "folder") folderMap.set(target.path, target);
    else documentMap.set(target.path, target);
  }

  const folders = Array.from(folderMap.values())
    .sort((left, right) => left.path.split("/").length - right.path.split("/").length);

  const keptFolders: KnowledgeDeleteTarget[] = [];
  for (const folder of folders) {
    if (keptFolders.some((entry) => isDescendantPath(folder.path, entry.path))) continue;
    keptFolders.push(folder);
  }

  const keptDocuments = Array.from(documentMap.values())
    .filter((document) =>
      !keptFolders.some((folder) => isDescendantPath(document.path, folder.path))
    );

  return [...keptFolders, ...keptDocuments];
}
