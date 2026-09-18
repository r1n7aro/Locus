export type GitGraphSelectionTarget =
  | { kind: "commit"; hash: string }
  | { kind: "stash"; hash: string }
  | { kind: "workspace" };

export interface GitGraphSelectOptions {
  scroll?: boolean;
  toggle?: boolean;
  behavior?: ScrollBehavior;
}

export interface GitGraphScrollOptions {
  behavior?: ScrollBehavior;
}

export interface GitGraphPublicApi {
  /** Known off-page refs/stashes remain selectable; returns false if they cannot be revealed. */
  selectHistory(target: GitGraphSelectionTarget, options?: GitGraphSelectOptions): Promise<boolean>;
  scrollToHistory(target: GitGraphSelectionTarget, options?: GitGraphScrollOptions): Promise<boolean>;
}
