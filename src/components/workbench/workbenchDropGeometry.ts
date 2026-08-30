import type { WorkbenchDropDirection } from "../../types/workbench";

export type WorkbenchSplitDropDirection = Exclude<WorkbenchDropDirection, "center">;

export interface WorkbenchDropPoint {
  x: number;
  y: number;
}

export interface WorkbenchDropBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export interface WorkbenchTabDropBounds {
  left: number;
  right: number;
}

export { tabInsertionIndexAtPoint as workbenchTabInsertionIndexAtPoint } from "../ui/tabDropGeometry";

function unitPosition(value: number, start: number, end: number): number {
  const size = Math.max(1, end - start);
  return Math.min(1, Math.max(0, (value - start) / size));
}

/**
 * Divide an editor body into four triangular regions meeting at its center.
 * The selected region always previews one equal half of the target group.
 */
export function workbenchSplitDirectionAtPoint(
  point: WorkbenchDropPoint,
  bounds: WorkbenchDropBounds,
): WorkbenchSplitDropDirection {
  const x = unitPosition(point.x, bounds.left, bounds.right);
  const y = unitPosition(point.y, bounds.top, bounds.bottom);
  const horizontalDistance = Math.abs(x - 0.5);
  const verticalDistance = Math.abs(y - 0.5);
  if (horizontalDistance >= verticalDistance) return x < 0.5 ? "left" : "right";
  return y < 0.5 ? "top" : "bottom";
}
