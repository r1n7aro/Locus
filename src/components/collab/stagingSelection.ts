export interface ResolveStagingFileSelectionInput {
  visiblePaths: string[];
  selectedPaths: Set<string>;
  lastClickedPath: string | null;
  clickedPath: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}

export interface ResolveStagingFileSelectionResult {
  nextSelectedPaths: Set<string>;
  nextLastClickedPath: string | null;
  shouldActivateFile: boolean;
}

export interface ResolveStagingFolderSelectionInput {
  selectedPaths: Set<string>;
  lastClickedPath: string | null;
  folderPaths: readonly string[];
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}

export interface ResolveStagingFolderSelectionResult {
  nextSelectedPaths: Set<string>;
  nextLastClickedPath: string | null;
}

export function resolveStagingFileSelection(
  input: ResolveStagingFileSelectionInput,
): ResolveStagingFileSelectionResult {
  const {
    visiblePaths,
    selectedPaths,
    lastClickedPath,
    clickedPath,
    shiftKey,
    ctrlKey,
    metaKey,
  } = input;
  const clickedIndex = visiblePaths.indexOf(clickedPath);
  const anchorIndex = lastClickedPath === null ? null : visiblePaths.indexOf(lastClickedPath);

  if (clickedIndex < 0) {
    return {
      nextSelectedPaths: new Set(selectedPaths),
      nextLastClickedPath: lastClickedPath,
      shouldActivateFile: false,
    };
  }

  if (shiftKey) {
    if (anchorIndex !== null && anchorIndex >= 0) {
      const [start, end] = anchorIndex <= clickedIndex
        ? [anchorIndex, clickedIndex]
        : [clickedIndex, anchorIndex];
      const next = new Set(selectedPaths);
      for (let i = start; i <= end; i++) {
        next.add(visiblePaths[i]);
      }
      return {
        nextSelectedPaths: next,
        nextLastClickedPath: clickedPath,
        shouldActivateFile: false,
      };
    }

    return {
      nextSelectedPaths: new Set([clickedPath]),
      nextLastClickedPath: clickedPath,
      shouldActivateFile: false,
    };
  }

  if (ctrlKey || metaKey) {
    const next = new Set(selectedPaths);
    if (next.has(clickedPath)) {
      next.delete(clickedPath);
    } else {
      next.add(clickedPath);
    }
    return {
      nextSelectedPaths: next,
      nextLastClickedPath: clickedPath,
      shouldActivateFile: false,
    };
  }

  return {
    nextSelectedPaths: new Set(),
    nextLastClickedPath: clickedPath,
    shouldActivateFile: true,
  };
}

export function resolveStagingFolderSelection(
  input: ResolveStagingFolderSelectionInput,
): ResolveStagingFolderSelectionResult {
  const {
    selectedPaths,
    lastClickedPath,
    folderPaths,
    shiftKey,
    ctrlKey,
    metaKey,
  } = input;
  if (folderPaths.length === 0) {
    return {
      nextSelectedPaths: new Set(selectedPaths),
      nextLastClickedPath: lastClickedPath,
    };
  }

  const next = new Set(selectedPaths);
  if (shiftKey) {
    for (const path of folderPaths) {
      next.add(path);
    }
  } else if (ctrlKey || metaKey) {
    const allSelected = folderPaths.every((path) => next.has(path));
    for (const path of folderPaths) {
      if (allSelected) {
        next.delete(path);
      } else {
        next.add(path);
      }
    }
  }

  return {
    nextSelectedPaths: next,
    nextLastClickedPath: folderPaths[folderPaths.length - 1],
  };
}
