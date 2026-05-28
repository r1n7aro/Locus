import type { ImageAttachment } from "../../types";
import type {
  GraphData,
  GraphLayoutDirection,
  GraphLink,
  GraphNode,
  GraphPoint,
  GraphStatePortPlacement,
} from "./graphTypes";
import {
  estimateGraphNodeHeightForStyle,
  estimateGraphNodeWidthForStyle,
  graphConnections,
  graphNodePortAnchor,
  graphRoutePointsWithAnchors,
  graphStateNodePortSide,
  normalizeGraphNodeStyle,
  normalizeGraphStatePortPlacement,
  normalizeGraphData,
} from "./graphTypes";

const GRAPH_IMAGE_MAX_WIDTH = 1800;
const GRAPH_IMAGE_MAX_HEIGHT = 1200;
const GRAPH_IMAGE_MIN_WIDTH = 520;
const GRAPH_IMAGE_MIN_HEIGHT = 320;
const GRAPH_IMAGE_PADDING = 48;
const GRAPH_IMAGE_EDGE_COLORS = [
  "#7c8cff",
  "#d5a843",
  "#50b98c",
  "#d87070",
  "#8eb9ff",
  "#b98cff",
] as const;

interface GraphImageBounds {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

interface GraphDirectionMarker {
  x: number;
  y: number;
  angle: number;
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function svgNumber(value: number): string {
  if (Number.isInteger(value)) return String(value);
  return value.toFixed(2).replace(/\.?0+$/, "");
}

function escapeSvgText(value: unknown): string {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function escapeSvgAttr(value: unknown): string {
  return escapeSvgText(value).replace(/"/g, "&quot;");
}

function compactText(value: unknown, maxLength: number): string {
  const text = String(value ?? "").trim();
  if (text.length <= maxLength) return text;
  return `${text.slice(0, Math.max(0, maxLength - 1))}...`;
}

function nodeWidth(node: GraphNode): number {
  return Math.round(node.width ?? estimateGraphNodeWidthForStyle(node));
}

function nodeHeight(node: GraphNode): number {
  return Math.round(node.height ?? estimateGraphNodeHeightForStyle(node));
}

function mergeBounds(bounds: GraphImageBounds, point: GraphPoint) {
  if (!finiteNumber(point.x) || !finiteNumber(point.y)) return;
  bounds.left = Math.min(bounds.left, point.x);
  bounds.top = Math.min(bounds.top, point.y);
  bounds.right = Math.max(bounds.right, point.x);
  bounds.bottom = Math.max(bounds.bottom, point.y);
}

function graphBounds(graph: GraphData): GraphImageBounds | null {
  if (!graph.nodes.length) return null;
  const bounds: GraphImageBounds = {
    left: Number.POSITIVE_INFINITY,
    top: Number.POSITIVE_INFINITY,
    right: Number.NEGATIVE_INFINITY,
    bottom: Number.NEGATIVE_INFINITY,
  };

  for (const node of graph.nodes) {
    const x = finiteNumber(node.x) ? node.x : 0;
    const y = finiteNumber(node.y) ? node.y : 0;
    mergeBounds(bounds, { x, y });
    mergeBounds(bounds, { x: x + nodeWidth(node), y: y + nodeHeight(node) });
  }

  for (const connection of graphConnections(graph)) {
    for (const point of connection.points ?? []) {
      mergeBounds(bounds, point);
    }
  }

  if (!Number.isFinite(bounds.left) || !Number.isFinite(bounds.top)) return null;
  return bounds;
}

function connectionColor(connection: GraphLink, index: number): string {
  const colorIndex = connection.style?.colorIndex;
  const normalized = finiteNumber(colorIndex) ? Math.abs(Math.trunc(colorIndex)) : index;
  return GRAPH_IMAGE_EDGE_COLORS[normalized % GRAPH_IMAGE_EDGE_COLORS.length];
}

function graphDistance(a: GraphPoint, b: GraphPoint): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

function compactGraphPoints(points: GraphPoint[]): GraphPoint[] {
  const result: GraphPoint[] = [];
  for (const point of points) {
    if (!finiteNumber(point.x) || !finiteNumber(point.y)) continue;
    const previous = result[result.length - 1];
    if (previous && graphDistance(previous, point) < 0.5) continue;
    result.push(point);
  }
  return result;
}

function bezierControls(start: GraphPoint, end: GraphPoint, vertical = false) {
  if (vertical) {
    const dy = Math.max(56, Math.abs(end.y - start.y) * 0.5);
    const offsetY = end.y >= start.y ? dy : -dy;
    return {
      controlA: { x: start.x, y: start.y + offsetY },
      controlB: { x: end.x, y: end.y - offsetY },
    };
  }

  const dx = Math.max(56, Math.abs(end.x - start.x) * 0.5);
  const offsetX = end.x >= start.x ? dx : -dx;
  return {
    controlA: { x: start.x + offsetX, y: start.y },
    controlB: { x: end.x - offsetX, y: end.y },
  };
}

function bezierPath(start: GraphPoint, end: GraphPoint, vertical = false): string {
  const { controlA, controlB } = bezierControls(start, end, vertical);
  return [
    "M", svgNumber(start.x), svgNumber(start.y),
    "C", svgNumber(controlA.x), svgNumber(controlA.y),
    svgNumber(controlB.x), svgNumber(controlB.y),
    svgNumber(end.x), svgNumber(end.y),
  ].join(" ");
}

function routePath(points: GraphPoint[]): string {
  return points
    .map((point, index) => {
      const command = index === 0 ? "M" : "L";
      return `${command} ${svgNumber(point.x)} ${svgNumber(point.y)}`;
    })
    .join(" ");
}

function cubicPoint(
  start: GraphPoint,
  controlA: GraphPoint,
  controlB: GraphPoint,
  end: GraphPoint,
  t: number,
): GraphPoint {
  const mt = 1 - t;
  return {
    x: mt ** 3 * start.x + 3 * mt ** 2 * t * controlA.x + 3 * mt * t ** 2 * controlB.x + t ** 3 * end.x,
    y: mt ** 3 * start.y + 3 * mt ** 2 * t * controlA.y + 3 * mt * t ** 2 * controlB.y + t ** 3 * end.y,
  };
}

function cubicTangent(
  start: GraphPoint,
  controlA: GraphPoint,
  controlB: GraphPoint,
  end: GraphPoint,
  t: number,
): GraphPoint {
  const mt = 1 - t;
  return {
    x: 3 * mt ** 2 * (controlA.x - start.x)
      + 6 * mt * t * (controlB.x - controlA.x)
      + 3 * t ** 2 * (end.x - controlB.x),
    y: 3 * mt ** 2 * (controlA.y - start.y)
      + 6 * mt * t * (controlB.y - controlA.y)
      + 3 * t ** 2 * (end.y - controlB.y),
  };
}

function directionMarkerFromBezier(start: GraphPoint, end: GraphPoint, vertical = false): GraphDirectionMarker | null {
  const { controlA, controlB } = bezierControls(start, end, vertical);
  const point = cubicPoint(start, controlA, controlB, end, 0.5);
  const tangent = cubicTangent(start, controlA, controlB, end, 0.5);
  if (Math.hypot(tangent.x, tangent.y) < 0.5) return null;
  return {
    x: point.x,
    y: point.y,
    angle: Math.atan2(tangent.y, tangent.x),
  };
}

function directionMarkerFromPoints(points: GraphPoint[]): GraphDirectionMarker | null {
  const route = compactGraphPoints(points);
  if (route.length < 2) return null;

  let total = 0;
  for (let index = 1; index < route.length; index += 1) {
    total += graphDistance(route[index - 1], route[index]);
  }
  if (total < 1) return null;

  const target = total / 2;
  let offset = 0;
  for (let index = 1; index < route.length; index += 1) {
    const from = route[index - 1];
    const to = route[index];
    const segmentLength = graphDistance(from, to);
    if (segmentLength < 1) continue;
    if (offset + segmentLength >= target) {
      const t = (target - offset) / segmentLength;
      return {
        x: from.x + (to.x - from.x) * t,
        y: from.y + (to.y - from.y) * t,
        angle: Math.atan2(to.y - from.y, to.x - from.x),
      };
    }
    offset += segmentLength;
  }

  const previous = route[route.length - 2];
  const last = route[route.length - 1];
  return {
    x: last.x,
    y: last.y,
    angle: Math.atan2(last.y - previous.y, last.x - previous.x),
  };
}

function directionChevronPath(marker: GraphDirectionMarker): string {
  const length = 9;
  const spread = 4.5;
  const forward = { x: Math.cos(marker.angle), y: Math.sin(marker.angle) };
  const normal = { x: -forward.y, y: forward.x };
  const apex = {
    x: marker.x + forward.x * length * 0.5,
    y: marker.y + forward.y * length * 0.5,
  };
  const tail = {
    x: marker.x - forward.x * length * 0.5,
    y: marker.y - forward.y * length * 0.5,
  };
  const left = {
    x: tail.x + normal.x * spread,
    y: tail.y + normal.y * spread,
  };
  const right = {
    x: tail.x - normal.x * spread,
    y: tail.y - normal.y * spread,
  };
  return [
    "M", svgNumber(left.x), svgNumber(left.y),
    "L", svgNumber(apex.x), svgNumber(apex.y),
    "M", svgNumber(right.x), svgNumber(right.y),
    "L", svgNumber(apex.x), svgNumber(apex.y),
  ].join(" ");
}

function connectionIsDirected(graph: GraphData, connection: GraphLink): boolean {
  return connection.directed ?? graph.layout?.directed ?? false;
}

function connectionUsesVerticalPorts(
  graph: GraphData,
  connection: GraphLink,
  direction: GraphLayoutDirection,
  statePortPlacement: GraphStatePortPlacement,
): boolean {
  const from = graph.nodes.find((node) => node.id === connection.from.nodeId);
  const to = graph.nodes.find((node) => node.id === connection.to.nodeId);
  const fromSide = from && normalizeGraphNodeStyle(from.nodeStyle) === "state"
    ? graphStateNodePortSide("output", direction, statePortPlacement)
    : "right";
  const toSide = to && normalizeGraphNodeStyle(to.nodeStyle) === "state"
    ? graphStateNodePortSide("input", direction, statePortPlacement)
    : "left";
  return fromSide === "top" || fromSide === "bottom" || toSide === "top" || toSide === "bottom";
}

function renderConnection(
  graph: GraphData,
  connection: GraphLink,
  index: number,
  direction: GraphLayoutDirection,
  statePortPlacement: GraphStatePortPlacement,
): string {
  const from = graph.nodes.find((node) => node.id === connection.from.nodeId);
  const to = graph.nodes.find((node) => node.id === connection.to.nodeId);
  if (!from || !to) return "";

  const start = graphNodePortAnchor(from, "output", connection.from.portId, direction, statePortPlacement);
  const end = graphNodePortAnchor(to, "input", connection.to.portId, direction, statePortPlacement);
  const routePoints = connection.points && connection.points.length >= 2
    ? graphRoutePointsWithAnchors(connection.points, start, end, direction)
    : null;
  const vertical = connectionUsesVerticalPorts(graph, connection, direction, statePortPlacement);
  const path = routePoints ? routePath(routePoints) : bezierPath(start, end, vertical);
  const color = connectionColor(connection, index);
  const directionMarker = connectionIsDirected(graph, connection)
    ? routePoints
      ? directionMarkerFromPoints(routePoints)
      : directionMarkerFromBezier(start, end, vertical)
    : null;
  const directionPath = directionMarker ? directionChevronPath(directionMarker) : "";
  return [
    `<path d="${escapeSvgAttr(path)}" fill="none" stroke="${color}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" opacity="0.88"/>`,
    directionPath
      ? `<path d="${escapeSvgAttr(directionPath)}" fill="none" stroke="${color}" stroke-width="1.55" stroke-linecap="round" stroke-linejoin="round" opacity="0.58"/>`
      : "",
  ].join("");
}

function renderNode(
  node: GraphNode,
  direction: GraphLayoutDirection,
  statePortPlacement: GraphStatePortPlacement,
): string {
  if (normalizeGraphNodeStyle(node.nodeStyle) === "state") {
    return renderStateNode(node, direction, statePortPlacement);
  }

  const x = finiteNumber(node.x) ? node.x : 0;
  const y = finiteNumber(node.y) ? node.y : 0;
  const width = nodeWidth(node);
  const height = nodeHeight(node);
  const title = escapeSvgText(compactText(node.title || node.id, 34));
  const subtitle = escapeSvgText(compactText(node.subtitle || node.type || "", 40));
  const inputCount = node.inputs?.length ?? 0;
  const outputCount = node.outputs?.length ?? 0;
  const parameterCount = node.parameters?.length ?? 0;
  const meta = [
    inputCount ? `${inputCount} in` : "",
    outputCount ? `${outputCount} out` : "",
    parameterCount ? `${parameterCount} params` : "",
  ].filter(Boolean).join(" / ");
  const metaText = escapeSvgText(meta);
  const subtitleText = subtitle || metaText;

  return [
    `<g transform="translate(${svgNumber(x)} ${svgNumber(y)})">`,
    `<rect x="0" y="0" width="${svgNumber(width)}" height="${svgNumber(height)}" rx="8" fill="#171a20" stroke="#343944" stroke-width="1"/>`,
    `<rect x="0" y="0" width="${svgNumber(width)}" height="42" rx="8" fill="#1d2129"/>`,
    `<path d="M 0 34 L 0 42 L ${svgNumber(width)} 42 L ${svgNumber(width)} 34" fill="#1d2129"/>`,
    `<text x="16" y="20" fill="#f3f6fb" font-size="13" font-weight="650" font-family="Segoe UI, Arial, sans-serif">${title}</text>`,
    subtitleText
      ? `<text x="16" y="36" fill="#9fa8b8" font-size="10.5" font-family="Segoe UI, Arial, sans-serif">${subtitleText}</text>`
      : "",
    `<circle cx="0" cy="21" r="4.5" fill="#7c8cff" stroke="#0f1116" stroke-width="2"/>`,
    `<circle cx="${svgNumber(width)}" cy="21" r="4.5" fill="#7c8cff" stroke="#0f1116" stroke-width="2"/>`,
    "</g>",
  ].join("");
}

function renderStateNode(
  node: GraphNode,
  direction: GraphLayoutDirection,
  statePortPlacement: GraphStatePortPlacement,
): string {
  const x = finiteNumber(node.x) ? node.x : 0;
  const y = finiteNumber(node.y) ? node.y : 0;
  const width = nodeWidth(node);
  const height = nodeHeight(node);
  const title = escapeSvgText(compactText(node.title || node.id, 28));
  const subtitle = escapeSvgText(compactText(node.subtitle || node.type || "", 34));
  const centerY = Math.round(height / 2);
  const titleY = subtitle ? centerY - 3 : centerY + 4;
  const subtitleY = centerY + 14;
  const portRadius = 4.5;
  const inputSide = graphStateNodePortSide("input", direction, statePortPlacement);
  const outputSide = graphStateNodePortSide("output", direction, statePortPlacement);
  const portCircle = (side: ReturnType<typeof graphStateNodePortSide>) => {
    if (side === "top") {
      return `<circle cx="${svgNumber(width / 2)}" cy="0" r="${svgNumber(portRadius)}" fill="#7c8cff" stroke="#101217" stroke-width="2"/>`;
    }
    if (side === "bottom") {
      return `<circle cx="${svgNumber(width / 2)}" cy="${svgNumber(height)}" r="${svgNumber(portRadius)}" fill="#7c8cff" stroke="#101217" stroke-width="2"/>`;
    }
    if (side === "right") {
      return `<circle cx="${svgNumber(width)}" cy="${svgNumber(centerY)}" r="${svgNumber(portRadius)}" fill="#7c8cff" stroke="#101217" stroke-width="2"/>`;
    }
    return `<circle cx="0" cy="${svgNumber(centerY)}" r="${svgNumber(portRadius)}" fill="#7c8cff" stroke="#101217" stroke-width="2"/>`;
  };

  return [
    `<g transform="translate(${svgNumber(x)} ${svgNumber(y)})">`,
    `<rect x="0" y="0" width="${svgNumber(width)}" height="${svgNumber(height)}" rx="12" fill="#1a1d24" stroke="#414958" stroke-width="1.25"/>`,
    `<path d="M 10 1 H ${svgNumber(width - 10)} Q ${svgNumber(width - 1)} 1 ${svgNumber(width - 1)} 10" fill="none" stroke="#5a6578" stroke-width="1" opacity="0.55"/>`,
    `<text x="${svgNumber(width / 2)}" y="${svgNumber(titleY)}" text-anchor="middle" fill="#f3f6fb" font-size="13" font-weight="650" font-family="Segoe UI, Arial, sans-serif">${title}</text>`,
    subtitle
      ? `<text x="${svgNumber(width / 2)}" y="${svgNumber(subtitleY)}" text-anchor="middle" fill="#9fa8b8" font-size="10.5" font-family="Segoe UI, Arial, sans-serif">${subtitle}</text>`
      : "",
    portCircle(inputSide),
    portCircle(outputSide),
    "</g>",
  ].join("");
}

function buildGraphSvg(graph: GraphData): { svg: string; width: number; height: number } | null {
  const bounds = graphBounds(graph);
  if (!bounds) return null;

  const viewLeft = bounds.left - GRAPH_IMAGE_PADDING;
  const viewTop = bounds.top - GRAPH_IMAGE_PADDING;
  const viewWidth = Math.max(1, bounds.right - bounds.left + GRAPH_IMAGE_PADDING * 2);
  const viewHeight = Math.max(1, bounds.bottom - bounds.top + GRAPH_IMAGE_PADDING * 2);
  const scale = Math.min(1, GRAPH_IMAGE_MAX_WIDTH / viewWidth, GRAPH_IMAGE_MAX_HEIGHT / viewHeight);
  const width = Math.max(GRAPH_IMAGE_MIN_WIDTH, Math.ceil(viewWidth * scale));
  const height = Math.max(GRAPH_IMAGE_MIN_HEIGHT, Math.ceil(viewHeight * scale));
  const direction = graph.layout?.direction ?? "right";
  const statePortPlacement = normalizeGraphStatePortPlacement(graph.layout?.statePortPlacement);
  const links = graphConnections(graph)
    .map((connection, index) => renderConnection(graph, connection, index, direction, statePortPlacement))
    .join("");
  const nodes = graph.nodes.map((node) => renderNode(node, direction, statePortPlacement)).join("");

  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="${svgNumber(viewLeft)} ${svgNumber(viewTop)} ${svgNumber(viewWidth)} ${svgNumber(viewHeight)}">`,
    `<rect x="${svgNumber(viewLeft)}" y="${svgNumber(viewTop)}" width="${svgNumber(viewWidth)}" height="${svgNumber(viewHeight)}" fill="#101217"/>`,
    `<g>${links}</g>`,
    `<g>${nodes}</g>`,
    "</svg>",
  ].join("");
  return { svg, width, height };
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("Graph layout image failed to render."));
    image.src = src;
  });
}

export async function renderGraphPngAttachment(graph: GraphData): Promise<ImageAttachment | null> {
  if (typeof document === "undefined") return null;
  const normalized = normalizeGraphData(graph);
  const built = buildGraphSvg(normalized);
  if (!built) return null;

  const image = await loadImage(`data:image/svg+xml;charset=utf-8,${encodeURIComponent(built.svg)}`);
  const canvas = document.createElement("canvas");
  canvas.width = built.width;
  canvas.height = built.height;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("Graph layout image canvas is unavailable.");
  }
  context.drawImage(image, 0, 0, built.width, built.height);
  const dataUrl = canvas.toDataURL("image/png");
  const data = dataUrl.slice(dataUrl.indexOf(",") + 1);
  return {
    data,
    mimeType: "image/png",
  };
}
