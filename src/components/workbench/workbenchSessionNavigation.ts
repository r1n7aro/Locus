export type WorkbenchSessionNavigationMode = "activate" | "reuse" | "newTab";

export type WorkbenchNewSessionShortcutAction = "keepCurrent" | "replaceCurrent" | "newTab";

export interface WorkbenchNewSessionShortcutContext {
  currentIsNewSession: boolean;
  tabStripVisible: boolean;
}

export interface WorkbenchSessionNavigationContext {
  targetOpen: boolean;
  splitLayout: boolean;
  focusedGroupTabCount: number;
  currentEditorProtected?: boolean;
}

export function workbenchSessionNavigationMode(
  context: WorkbenchSessionNavigationContext,
): WorkbenchSessionNavigationMode {
  if (context.targetOpen) return "activate";
  if (
    context.splitLayout
    || context.focusedGroupTabCount > 1
    || context.currentEditorProtected
    || context.focusedGroupTabCount === 0
  ) return "newTab";
  return "reuse";
}

export function workbenchNewSessionShortcutAction(
  context: WorkbenchNewSessionShortcutContext,
): WorkbenchNewSessionShortcutAction {
  if (context.currentIsNewSession) return "keepCurrent";
  return context.tabStripVisible ? "newTab" : "replaceCurrent";
}
