import type { InspectorField } from "../../types";

export const AUTO_COLLAPSE_CHILD_COUNT = 10;

export function shouldAutoCollapseField(field: InspectorField): boolean {
  return (field.children?.length ?? 0) > AUTO_COLLAPSE_CHILD_COUNT;
}
