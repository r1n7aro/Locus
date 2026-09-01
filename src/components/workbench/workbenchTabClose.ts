export type WorkbenchTabCloseScope = "current" | "left" | "right" | "all";

export function workbenchTabCloseIds(
  tabIds: readonly string[],
  targetId: string,
  scope: WorkbenchTabCloseScope,
): string[] {
  const targetIndex = tabIds.indexOf(targetId);
  if (targetIndex < 0) return [];
  switch (scope) {
    case "current":
      return [targetId];
    case "left":
      return tabIds.slice(0, targetIndex);
    case "right":
      return tabIds.slice(targetIndex + 1);
    case "all":
      return [...tabIds];
  }
}
