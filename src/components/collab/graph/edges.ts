import type { HistoryGraphEdgeLayout } from "./types";

interface Point { x: number; y: number }

export interface HistoryGraphEdgeRoute extends Omit<HistoryGraphEdgeLayout, "path" | "crossings"> {
  start: Point;
  end: Point;
}

function curvePoint(start: Point, end: Point, t: number): Point {
  const easedX = t * t * (3 - 2 * t);
  const easedY = t * (1.5 - 1.5 * t + t * t);
  return { x: start.x + (end.x - start.x) * easedX, y: start.y + (end.y - start.y) * easedY };
}

function sampleRoute(route: HistoryGraphEdgeRoute): Point[] {
  if (route.start.x === route.end.x) return [route.start, route.end];
  // Subpixel approximation of the cubic, including wide octopus merges.
  const steps = Math.max(24, Math.ceil(Math.abs(route.end.x - route.start.x) / 4));
  return Array.from({ length: steps + 1 }, (_, index) => curvePoint(route.start, route.end, index / steps));
}

function segmentCrossing(a: Point, b: Point, c: Point, d: Point): Point | null {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const ex = d.x - c.x;
  const ey = d.y - c.y;
  const determinant = dx * ey - dy * ex;
  if (Math.abs(determinant) < 1e-8) return null;
  const t = ((c.x - a.x) * ey - (c.y - a.y) * ex) / determinant;
  const u = ((c.x - a.x) * dy - (c.y - a.y) * dx) / determinant;
  if (t < 0 || t > 1 || u < 0 || u > 1) return null;
  return { x: a.x + t * dx, y: a.y + t * dy };
}

function routeCrossings(left: Point[], right: Point[]): Point[] {
  const crossings: Point[] = [];
  let i = 0;
  let j = 0;
  const top = Math.max(left[0]!.y, right[0]!.y);
  const bottom = Math.min(left[left.length - 1]!.y, right[right.length - 1]!.y);
  while (i < left.length - 1 && j < right.length - 1) {
    const a = left[i]!;
    const b = left[i + 1]!;
    const c = right[j]!;
    const d = right[j + 1]!;
    if (b.y >= c.y && d.y >= a.y) {
      const point = segmentCrossing(a, b, c, d);
      // Shared endpoints are real graph junctions, never underpasses.
      if (point && point.y > top + 0.5 && point.y < bottom - 0.5
        && !crossings.some(previous => Math.hypot(previous.x - point.x, previous.y - point.y) < 0.5)) {
        crossings.push(point);
      }
    }
    if (b.y <= d.y) i++;
    if (d.y <= b.y) j++;
  }
  return crossings;
}

/** Preserve topology when row order makes a crossing unavoidable. */
export function layoutHistoryGraphEdges(routes: HistoryGraphEdgeRoute[]): HistoryGraphEdgeLayout[] {
  const edges = routes.map(({ start, end, ...edge }) => {
    const midY = (start.y + end.y) / 2;
    return {
      ...edge,
      path: start.x === end.x
        ? `M${start.x},${start.y}L${end.x},${end.y}`
        : `M${start.x},${start.y}C${start.x},${midY} ${end.x},${midY} ${end.x},${end.y}`,
    } as HistoryGraphEdgeLayout;
  });
  const ordered = routes.map((route, index) => ({ route, index, points: sampleRoute(route) }))
    .sort((left, right) => left.route.start.y - right.route.start.y || left.index - right.index);
  let active: typeof ordered = [];
  for (const current of ordered) {
    active = active.filter(previous => previous.route.end.y > current.route.start.y);
    for (const previous of active) {
      const left = previous.route;
      const right = current.route;
      if (Math.max(left.start.x, left.end.x) < Math.min(right.start.x, right.end.x)
        || Math.max(right.start.x, right.end.x) < Math.min(left.start.x, left.end.x)) continue;
      const crossings = routeCrossings(previous.points, current.points);
      if (crossings.length === 0) continue;
      // Keep straight tracks intact; the turning edge passes underneath.
      const under = right.start.x === right.end.x ? previous.index : current.index;
      (edges[under]!.crossings ??= []).push(...crossings);
    }
    active.push(current);
  }
  return edges;
}
