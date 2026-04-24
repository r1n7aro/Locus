export interface FloatingPoint {
  x: number;
  y: number;
}

export interface FloatingSize {
  width: number;
  height: number;
}

export function clampFloatingPosition(
  point: FloatingPoint,
  size: FloatingSize,
  viewport: FloatingSize,
  margin = 8,
): FloatingPoint {
  const safeMargin = Math.max(0, margin);
  const maxX = Math.max(safeMargin, viewport.width - Math.max(0, size.width) - safeMargin);
  const maxY = Math.max(safeMargin, viewport.height - Math.max(0, size.height) - safeMargin);

  return {
    x: Math.min(Math.max(point.x, safeMargin), maxX),
    y: Math.min(Math.max(point.y, safeMargin), maxY),
  };
}
