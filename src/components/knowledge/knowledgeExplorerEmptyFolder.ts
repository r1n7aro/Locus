export interface KnowledgeEmptyFolderState {
  searchMode: boolean;
  expanded: boolean;
  directChildCount: number;
  contentsLoaded: boolean;
  contentsLoading: boolean;
  hasMoreContents: boolean;
  hasTransientChild: boolean;
}

export function shouldShowKnowledgeEmptyFolder(
  state: KnowledgeEmptyFolderState,
): boolean {
  return (
    !state.searchMode
    && state.expanded
    && state.directChildCount === 0
    && state.contentsLoaded
    && !state.contentsLoading
    && !state.hasMoreContents
    && !state.hasTransientChild
  );
}
