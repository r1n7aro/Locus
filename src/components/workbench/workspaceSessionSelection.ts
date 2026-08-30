export interface ResolveWorkspaceSessionSelectionInput {
  visibleSessionIds: string[];
  selectedSessionIds: Set<string>;
  anchorSessionId: string | null;
  activeSessionId: string | null;
  clickedSessionId: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}

export interface ResolveWorkspaceSessionSelectionResult {
  nextSelectedSessionIds: Set<string>;
  nextAnchorSessionId: string | null;
  shouldActivateSession: boolean;
}

export function resolveWorkspaceSessionSelection(
  input: ResolveWorkspaceSessionSelectionInput,
): ResolveWorkspaceSessionSelectionResult {
  const {
    visibleSessionIds,
    selectedSessionIds,
    anchorSessionId,
    activeSessionId,
    clickedSessionId,
    shiftKey,
    ctrlKey,
    metaKey,
  } = input;
  const clickedIndex = visibleSessionIds.indexOf(clickedSessionId);
  if (clickedIndex < 0) {
    return {
      nextSelectedSessionIds: new Set(selectedSessionIds),
      nextAnchorSessionId: anchorSessionId,
      shouldActivateSession: false,
    };
  }

  if (shiftKey) {
    const rangeAnchorId = anchorSessionId && visibleSessionIds.includes(anchorSessionId)
      ? anchorSessionId
      : activeSessionId && visibleSessionIds.includes(activeSessionId)
        ? activeSessionId
        : null;
    if (rangeAnchorId) {
      const anchorIndex = visibleSessionIds.indexOf(rangeAnchorId);
      const [start, end] = anchorIndex <= clickedIndex
        ? [anchorIndex, clickedIndex]
        : [clickedIndex, anchorIndex];
      return {
        nextSelectedSessionIds: new Set(visibleSessionIds.slice(start, end + 1)),
        nextAnchorSessionId: rangeAnchorId,
        shouldActivateSession: false,
      };
    }
  }

  if (ctrlKey || metaKey) {
    const next = new Set(selectedSessionIds);
    if (next.has(clickedSessionId)) {
      next.delete(clickedSessionId);
    } else {
      if (
        next.size === 0
        && activeSessionId
        && activeSessionId !== clickedSessionId
        && visibleSessionIds.includes(activeSessionId)
      ) {
        next.add(activeSessionId);
      }
      next.add(clickedSessionId);
    }
    return {
      nextSelectedSessionIds: next,
      nextAnchorSessionId: clickedSessionId,
      shouldActivateSession: false,
    };
  }

  return {
    nextSelectedSessionIds: new Set(),
    nextAnchorSessionId: clickedSessionId,
    shouldActivateSession: true,
  };
}

export function resolveWorkspaceSessionContextIds(input: {
  visibleSessionIds: string[];
  selectedSessionIds: Set<string>;
  targetSessionId: string;
}): string[] {
  if (input.selectedSessionIds.size > 1 && input.selectedSessionIds.has(input.targetSessionId)) {
    return input.visibleSessionIds.filter((id) => input.selectedSessionIds.has(id));
  }
  return [input.targetSessionId];
}
