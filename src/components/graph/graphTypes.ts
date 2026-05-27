export type GraphPortDirection = "input" | "output";
export type GraphParameterType = "string" | "text" | "number" | "boolean" | "select" | "color";
export type GraphLayoutDirection = "right" | "left" | "down" | "up";
export type GraphPortSide = "left" | "right" | "top" | "bottom";
export const GRAPH_LAYOUT_MODES = ["flow", "dependency", "state", "radial", "manual"] as const;
export type GraphLayoutMode = typeof GRAPH_LAYOUT_MODES[number];
export const GRAPH_NODE_STYLES = ["blueprint", "state"] as const;
export type GraphNodeStyle = typeof GRAPH_NODE_STYLES[number];
export const GRAPH_STATE_PORT_PLACEMENTS = ["auto", "horizontal", "vertical"] as const;
export type GraphStatePortPlacement = typeof GRAPH_STATE_PORT_PLACEMENTS[number];
export type GraphAutoLayoutMode = boolean | "missing" | "always" | "off";
export type GraphNodePortsConfig = boolean | {
  input?: boolean;
  output?: boolean;
};

export interface GraphPoint {
  x: number;
  y: number;
}

export interface GraphPort {
  id: string;
  label?: string;
  direction?: GraphPortDirection;
  type?: string;
  accepts?: string[];
}

export interface GraphParameterOption {
  label: string;
  value: string | number | boolean;
}

export interface GraphParameter {
  id: string;
  label?: string;
  type?: GraphParameterType;
  value?: unknown;
  options?: GraphParameterOption[];
  min?: number;
  max?: number;
  step?: number;
  placeholder?: string;
  readOnly?: boolean;
}

export interface GraphNode {
  id: string;
  type?: string;
  title?: string;
  subtitle?: string;
  nodeStyle?: GraphNodeStyle;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  inputs?: GraphPort[];
  outputs?: GraphPort[];
  parameters?: GraphParameter[];
  data?: unknown;
}

export interface GraphEndpoint {
  nodeId: string;
  portId?: string | null;
}

export interface GraphLinkStyle {
  colorIndex?: number;
  hasOverlap?: boolean;
  overlapGroupId?: string;
}

export interface GraphLink {
  id?: string;
  from: GraphEndpoint;
  to: GraphEndpoint;
  label?: string;
  type?: string;
  directed?: boolean;
  points?: GraphPoint[];
  style?: GraphLinkStyle;
  data?: unknown;
}

export interface GraphLayoutOptions {
  engine?: "elk" | "none";
  mode?: GraphLayoutMode;
  nodeStyle?: GraphNodeStyle;
  direction?: GraphLayoutDirection;
  directed?: boolean;
  statePortPlacement?: GraphStatePortPlacement;
  auto?: GraphAutoLayoutMode;
  nodePorts?: GraphNodePortsConfig;
  nodeSpacing?: number;
  layerSpacing?: number;
  padding?: number;
}

export interface GraphData {
  schema?: "locus.graph.v1" | string;
  nodes: GraphNode[];
  links?: GraphLink[];
  connections?: GraphLink[];
  edges?: GraphLink[];
  layout?: GraphLayoutOptions;
}

export type GraphConnectionValidation =
  | boolean
  | string
  | {
      ok: boolean;
      message?: string;
    };

export const GRAPH_WORLD_SIZE = 4096;
export const GRAPH_NODE_WIDTH = 240;
export const GRAPH_NODE_MIN_WIDTH = 220;
export const GRAPH_NODE_MAX_WIDTH = 420;
export const GRAPH_NODE_MIN_HEIGHT = 112;
export const GRAPH_STATE_NODE_WIDTH = 190;
export const GRAPH_STATE_NODE_MIN_WIDTH = 158;
export const GRAPH_STATE_NODE_MAX_WIDTH = 320;
export const GRAPH_STATE_NODE_HEIGHT = 68;
export const GRAPH_STATE_NODE_MIN_HEIGHT = 56;
export const GRAPH_NODE_PORT_ID = "__node__";
export const GRAPH_PORT_SIZE = 13;
export const GRAPH_NODE_HEADER_HEIGHT = 42;
export const GRAPH_NODE_BODY_PADDING_TOP = 8;
export const GRAPH_PORT_ROW_GAP = 6;
export const GRAPH_PORT_ROW_PITCH = GRAPH_PORT_SIZE + GRAPH_PORT_ROW_GAP;
export const GRAPH_EDGE_COLOR_COUNT = 6;
const GRAPH_ROUTE_FIXED_OVERLAP_TOLERANCE = 16;
const GRAPH_ROUTE_MIN_OVERLAP_LENGTH = 8;

interface GraphRouteSegment {
  connectionIndex: number;
  orientation: "horizontal" | "vertical";
  fixed: number;
  from: number;
  to: number;
}

function connectionKey(connection: GraphLink, index: number): string {
  return connection.id || `connection-${index + 1}`;
}

function connectionOutputGroupKey(connection: GraphLink): string {
  return `${connection.from.nodeId}:${connection.from.portId || GRAPH_NODE_PORT_ID}`;
}

function estimateTextWidth(text: string): number {
  let width = 0;
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    if (code >= 0x2e80) {
      width += 13;
    } else if (/[A-Z0-9_]/.test(char)) {
      width += 7.4;
    } else if (/\s/.test(char)) {
      width += 4;
    } else {
      width += 6.4;
    }
  }
  return Math.ceil(width);
}

function textValue(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function clampNodeWidth(width: number): number {
  return Math.min(GRAPH_NODE_MAX_WIDTH, Math.max(GRAPH_NODE_MIN_WIDTH, Math.ceil(width)));
}

function clampStateNodeWidth(width: number): number {
  return Math.min(GRAPH_STATE_NODE_MAX_WIDTH, Math.max(GRAPH_STATE_NODE_MIN_WIDTH, Math.ceil(width)));
}

export function graphConnections(data: Pick<GraphData, "connections" | "links" | "edges">): GraphLink[] {
  if (Array.isArray(data.connections)) return data.connections;
  if (Array.isArray(data.links)) return data.links;
  if (Array.isArray(data.edges)) return data.edges;
  return [];
}

export function normalizeGraphLayoutMode(value: unknown): GraphLayoutMode {
  if (typeof value !== "string") return "flow";
  const normalized = value.trim().toLowerCase().replace(/[\s_-]+/g, "");
  if (normalized === "dependency" || normalized === "dependencies" || normalized === "network" || normalized === "stress") {
    return "dependency";
  }
  if (normalized === "state" || normalized === "statemachine" || normalized === "machine" || normalized === "force") {
    return "state";
  }
  if (normalized === "radial" || normalized === "circle" || normalized === "hub") {
    return "radial";
  }
  if (normalized === "manual" || normalized === "position" || normalized === "positions" || normalized === "fixed") {
    return "manual";
  }
  return "flow";
}

export function normalizeGraphNodeStyle(value: unknown): GraphNodeStyle {
  if (typeof value !== "string") return "blueprint";
  const normalized = value.trim().toLowerCase().replace(/[\s_-]+/g, "");
  if (
    normalized === "state"
    || normalized === "statemachine"
    || normalized === "unity"
    || normalized === "unreal"
    || normalized === "animationstate"
  ) {
    return "state";
  }
  return "blueprint";
}

export function normalizeGraphStatePortPlacement(value: unknown): GraphStatePortPlacement {
  if (typeof value !== "string") return "auto";
  const normalized = value.trim().toLowerCase().replace(/[\s_-]+/g, "");
  if (normalized === "horizontal" || normalized === "side" || normalized === "sides") return "horizontal";
  if (normalized === "vertical" || normalized === "topbottom" || normalized === "topbottoms") return "vertical";
  return "auto";
}

export function cloneGraphData(graph: GraphData): GraphData {
  const connections = graphConnections(graph).map((connection) => ({
    ...connection,
    from: { ...connection.from },
    to: { ...connection.to },
    points: connection.points?.map((point) => ({ ...point })),
    style: connection.style ? { ...connection.style } : undefined,
  }));

  return {
    ...graph,
    nodes: graph.nodes.map((node) => ({
      ...node,
      inputs: node.inputs?.map((port) => ({ ...port })),
      outputs: node.outputs?.map((port) => ({ ...port })),
      parameters: node.parameters?.map((parameter) => ({
        ...parameter,
        options: parameter.options?.map((option) => ({ ...option })),
      })),
    })),
    connections,
    links: connections,
    edges: undefined,
  };
}

export function estimateGraphNodeWidth(node: GraphNode): number {
  const title = node.title || node.id;
  const subtitle = node.subtitle || node.type || "";
  const inputLabelWidth = Math.max(0, ...(node.inputs ?? []).map((port) => estimateTextWidth(port.label || port.id)));
  const outputLabelWidth = Math.max(0, ...(node.outputs ?? []).map((port) => estimateTextWidth(port.label || port.id)));
  const parameterLabelWidth = Math.max(0, ...(node.parameters ?? []).map((parameter) => estimateTextWidth(parameter.label || parameter.id)));
  const parameterValueWidth = Math.max(0, ...(node.parameters ?? []).map((parameter) => estimateTextWidth(textValue(parameter.value))));

  const headerWidth = Math.max(
    estimateTextWidth(title) + 92,
    subtitle ? estimateTextWidth(subtitle) + 92 : 0,
  );
  const portWidth = inputLabelWidth || outputLabelWidth
    ? inputLabelWidth + outputLabelWidth + 92
    : 0;
  const parameterWidth = parameterLabelWidth || parameterValueWidth
    ? parameterLabelWidth + Math.min(parameterValueWidth, 210) + 88
    : 0;

  return clampNodeWidth(Math.max(GRAPH_NODE_WIDTH, headerWidth, portWidth, parameterWidth));
}

export function estimateGraphNodeHeight(node: GraphNode): number {
  const inputCount = node.inputs?.length ?? 0;
  const outputCount = node.outputs?.length ?? 0;
  const portRows = Math.max(inputCount, outputCount);
  const parameterCount = node.parameters?.length ?? 0;
  const portsHeight = portRows > 0 ? portRows * 21 + 2 : 0;
  const parametersHeight = parameterCount > 0 ? parameterCount * 35 + 12 : 0;
  return Math.max(GRAPH_NODE_MIN_HEIGHT, 58 + portsHeight + parametersHeight);
}

export function estimateGraphStateNodeWidth(node: GraphNode): number {
  const title = node.title || node.id;
  const subtitle = node.subtitle || node.type || "";
  return clampStateNodeWidth(Math.max(
    GRAPH_STATE_NODE_WIDTH,
    estimateTextWidth(title) + 72,
    subtitle ? estimateTextWidth(subtitle) + 72 : 0,
  ));
}

export function estimateGraphStateNodeHeight(node: GraphNode): number {
  return Math.max(GRAPH_STATE_NODE_MIN_HEIGHT, node.subtitle || node.type ? GRAPH_STATE_NODE_HEIGHT : 60);
}

export function estimateGraphNodeWidthForStyle(
  node: GraphNode,
  style: GraphNodeStyle = normalizeGraphNodeStyle(node.nodeStyle),
): number {
  return style === "state"
    ? estimateGraphStateNodeWidth(node)
    : estimateGraphNodeWidth(node);
}

export function estimateGraphNodeHeightForStyle(
  node: GraphNode,
  style: GraphNodeStyle = normalizeGraphNodeStyle(node.nodeStyle),
): number {
  return style === "state"
    ? estimateGraphStateNodeHeight(node)
    : estimateGraphNodeHeight(node);
}

function normalizeGraphNodeWidth(node: GraphNode, style: GraphNodeStyle): number {
  if (typeof node.width === "number" && Number.isFinite(node.width)) {
    const minWidth = style === "state" ? GRAPH_STATE_NODE_MIN_WIDTH : GRAPH_NODE_MIN_WIDTH;
    if (Number(node.width) < minWidth) {
      return estimateGraphNodeWidthForStyle(node, style);
    }
    return style === "state"
      ? clampStateNodeWidth(Number(node.width))
      : clampNodeWidth(Number(node.width));
  }
  return estimateGraphNodeWidthForStyle(node, style);
}

function normalizeGraphNodeHeight(node: GraphNode, style: GraphNodeStyle): number {
  if (typeof node.height === "number" && Number.isFinite(node.height)) {
    return Math.max(Number(node.height), estimateGraphNodeHeightForStyle(node, style));
  }
  return estimateGraphNodeHeightForStyle(node, style);
}

export function normalizeGraphData(data: GraphData | null | undefined): GraphData {
  const source = data ?? { nodes: [], connections: [] };
  const nodes = Array.isArray(source.nodes) ? source.nodes : [];
  const connections = graphConnections(source);
  const normalizedConnections = connections
    .filter((connection) => connection.from?.nodeId && connection.to?.nodeId)
    .map((connection, index) => ({
      ...connection,
      id: connection.id || `connection-${index + 1}`,
      from: {
        nodeId: String(connection.from.nodeId),
        portId: connection.from.portId ?? null,
      },
      to: {
        nodeId: String(connection.to.nodeId),
        portId: connection.to.portId ?? null,
      },
      points: connection.points?.map((point) => ({ x: point.x, y: point.y })),
      style: connection.style ? { ...connection.style } : undefined,
    }));

  return {
    schema: source.schema ?? "locus.graph.v1",
    layout: source.layout ? { ...source.layout } : undefined,
    nodes: nodes.map((node, index) => {
      const styleSource = node.nodeStyle ?? source.layout?.nodeStyle;
      const nodeStyle = normalizeGraphNodeStyle(styleSource);
      const shouldPersistStyle = styleSource !== undefined;
      const styledNode = shouldPersistStyle ? { ...node, nodeStyle } : node;
      return {
        ...styledNode,
        id: String(node.id || `node-${index + 1}`),
        x: typeof node.x === "number" && Number.isFinite(node.x) ? Number(node.x) : undefined,
        y: typeof node.y === "number" && Number.isFinite(node.y) ? Number(node.y) : undefined,
        width: normalizeGraphNodeWidth(styledNode, nodeStyle),
        height: normalizeGraphNodeHeight(styledNode, nodeStyle),
        inputs: (node.inputs ?? []).map((port) => ({ ...port, direction: "input" })),
        outputs: (node.outputs ?? []).map((port) => ({ ...port, direction: "output" })),
        parameters: (node.parameters ?? []).map((parameter) => ({ ...parameter })),
      };
    }),
    connections: normalizedConnections,
    links: normalizedConnections,
  };
}

export function graphIsDag(data: Pick<GraphData, "nodes" | "connections" | "links" | "edges">): boolean {
  const nodeIds = new Set(data.nodes.map((node) => node.id));
  const outgoing = new Map<string, string[]>();
  for (const nodeId of nodeIds) {
    outgoing.set(nodeId, []);
  }

  for (const connection of graphConnections(data)) {
    const from = connection.from?.nodeId;
    const to = connection.to?.nodeId;
    if (!from || !to || !nodeIds.has(from) || !nodeIds.has(to)) continue;
    outgoing.get(from)?.push(to);
  }

  const state = new Map<string, 0 | 1 | 2>();
  const visit = (nodeId: string): boolean => {
    const current = state.get(nodeId) ?? 0;
    if (current === 1) return false;
    if (current === 2) return true;
    state.set(nodeId, 1);
    for (const next of outgoing.get(nodeId) ?? []) {
      if (!visit(next)) return false;
    }
    state.set(nodeId, 2);
    return true;
  };

  for (const nodeId of nodeIds) {
    if (!visit(nodeId)) return false;
  }
  return true;
}

export function graphPortOffsetY(
  node: Pick<GraphNode, "height" | "inputs" | "nodeStyle" | "outputs">,
  direction: GraphPortDirection,
  portId?: string | null,
): number {
  if (normalizeGraphNodeStyle(node.nodeStyle) === "state") {
    return Math.round((node.height ?? GRAPH_STATE_NODE_HEIGHT) / 2);
  }
  if (!portId) return GRAPH_NODE_HEADER_HEIGHT / 2;
  const ports = direction === "input" ? node.inputs ?? [] : node.outputs ?? [];
  const index = ports.findIndex((port) => port.id === portId);
  if (index < 0) return GRAPH_NODE_HEADER_HEIGHT / 2;
  return GRAPH_NODE_HEADER_HEIGHT
    + GRAPH_NODE_BODY_PADDING_TOP
    + GRAPH_PORT_SIZE / 2
    + index * GRAPH_PORT_ROW_PITCH;
}

function graphStatePortsUseVertical(
  layoutDirection: GraphLayoutDirection,
  placement: GraphStatePortPlacement,
): boolean {
  if (placement === "vertical") return true;
  if (placement === "horizontal") return false;
  return layoutDirection === "down" || layoutDirection === "up";
}

export function graphStateNodePortSide(
  direction: GraphPortDirection,
  layoutDirection: GraphLayoutDirection = "right",
  placement: GraphStatePortPlacement = "auto",
): GraphPortSide {
  const normalizedPlacement = normalizeGraphStatePortPlacement(placement);
  if (graphStatePortsUseVertical(layoutDirection, normalizedPlacement)) {
    if (layoutDirection === "up") return direction === "input" ? "bottom" : "top";
    return direction === "input" ? "top" : "bottom";
  }

  if (layoutDirection === "left") return direction === "input" ? "right" : "left";
  return direction === "input" ? "left" : "right";
}

export function graphNodePortAnchor(
  node: GraphNode,
  direction: GraphPortDirection,
  portId?: string | null,
  layoutDirection: GraphLayoutDirection = "right",
  statePortPlacement: GraphStatePortPlacement = "auto",
): GraphPoint {
  const width = node.width ?? estimateGraphNodeWidthForStyle(node);
  const height = node.height ?? estimateGraphNodeHeightForStyle(node);
  if (normalizeGraphNodeStyle(node.nodeStyle) === "state") {
    const side = graphStateNodePortSide(direction, layoutDirection, statePortPlacement);
    if (side === "top") {
      return {
        x: (node.x ?? 0) + Math.round(width / 2),
        y: node.y ?? 0,
      };
    }
    if (side === "bottom") {
      return {
        x: (node.x ?? 0) + Math.round(width / 2),
        y: (node.y ?? 0) + height,
      };
    }
    return {
      x: (node.x ?? 0) + (side === "right" ? width : 0),
      y: (node.y ?? 0) + Math.round(height / 2),
    };
  }

  return {
    x: (node.x ?? 0) + (direction === "output" ? width : 0),
    y: (node.y ?? 0) + graphPortOffsetY(node, direction, portId),
  };
}

function graphUsesHorizontalPortRoutes(direction: GraphLayoutDirection): boolean {
  return direction === "right" || direction === "left";
}

export function graphOrthogonalRoutePoints(
  start: GraphPoint,
  end: GraphPoint,
  direction: GraphLayoutDirection = "right",
  channel?: number,
): GraphPoint[] {
  if (graphUsesHorizontalPortRoutes(direction)) {
    const channelX = Math.round(channel ?? (start.x + end.x) / 2);
    return [
      { ...start },
      { x: channelX, y: start.y },
      { x: channelX, y: end.y },
      { ...end },
    ];
  }

  const channelY = Math.round(channel ?? (start.y + end.y) / 2);
  return [
    { ...start },
    { x: start.x, y: channelY },
    { x: end.x, y: channelY },
    { ...end },
  ];
}

export function graphRoutePointsWithAnchors(
  points: GraphPoint[] | undefined,
  start: GraphPoint,
  end: GraphPoint,
  direction: GraphLayoutDirection = "right",
): GraphPoint[] {
  const route = (points ?? [])
    .filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y))
    .map((point) => ({ ...point }));
  const horizontal = graphUsesHorizontalPortRoutes(direction);

  if (route.length < 4) {
    const channel = route.length >= 3
      ? horizontal ? route[1].x : route[1].y
      : undefined;
    return graphOrthogonalRoutePoints(start, end, direction, channel);
  }

  route[0] = { ...start };
  route[route.length - 1] = { ...end };
  if (horizontal) {
    route[1] = { ...route[1], y: start.y };
    route[route.length - 2] = { ...route[route.length - 2], y: end.y };
  } else {
    route[1] = { ...route[1], x: start.x };
    route[route.length - 2] = { ...route[route.length - 2], x: end.x };
  }
  return route;
}

function graphRouteSegments(connection: GraphLink, connectionIndex: number): GraphRouteSegment[] {
  const points = connection.points ?? [];
  const segments: GraphRouteSegment[] = [];
  for (let index = 0; index < points.length - 1; index += 1) {
    const start = points[index];
    const end = points[index + 1];
    if (!Number.isFinite(start.x) || !Number.isFinite(start.y) || !Number.isFinite(end.x) || !Number.isFinite(end.y)) {
      continue;
    }
    const dx = Math.abs(end.x - start.x);
    const dy = Math.abs(end.y - start.y);
    if (dx < 1 && dy < 1) continue;
    if (dy <= 1) {
      segments.push({
        connectionIndex,
        orientation: "horizontal",
        fixed: Math.round((start.y + end.y) / 2),
        from: Math.min(start.x, end.x),
        to: Math.max(start.x, end.x),
      });
    } else if (dx <= 1) {
      segments.push({
        connectionIndex,
        orientation: "vertical",
        fixed: Math.round((start.x + end.x) / 2),
        from: Math.min(start.y, end.y),
        to: Math.max(start.y, end.y),
      });
    }
  }
  return segments.filter((segment) => segment.to - segment.from >= 8);
}

function routeSegmentsOverlap(a: GraphRouteSegment, b: GraphRouteSegment): boolean {
  if (a.connectionIndex === b.connectionIndex) return false;
  if (a.orientation === b.orientation) {
    if (Math.abs(a.fixed - b.fixed) > GRAPH_ROUTE_FIXED_OVERLAP_TOLERANCE) return false;
    const overlap = Math.min(a.to, b.to) - Math.max(a.from, b.from);
    return overlap >= GRAPH_ROUTE_MIN_OVERLAP_LENGTH;
  }

  const horizontal = a.orientation === "horizontal" ? a : b;
  const vertical = a.orientation === "vertical" ? a : b;
  const crossesHorizontalRange = vertical.fixed >= horizontal.from - GRAPH_ROUTE_FIXED_OVERLAP_TOLERANCE
    && vertical.fixed <= horizontal.to + GRAPH_ROUTE_FIXED_OVERLAP_TOLERANCE;
  const crossesVerticalRange = horizontal.fixed >= vertical.from - GRAPH_ROUTE_FIXED_OVERLAP_TOLERANCE
    && horizontal.fixed <= vertical.to + GRAPH_ROUTE_FIXED_OVERLAP_TOLERANCE;
  return crossesHorizontalRange && crossesVerticalRange;
}

export function graphRouteColorIndexById(
  data: Pick<GraphData, "connections" | "links" | "edges">,
  colorCount = GRAPH_EDGE_COLOR_COUNT,
): Map<string, number> {
  const connections = graphConnections(data);
  const segments = connections.flatMap((connection, index) => graphRouteSegments(connection, index));
  const groupIds: string[] = [];
  const groupIndexById = new Map<string, number>();
  const groupIndexByConnectionIndex = connections.map((connection) => {
    const groupId = connectionOutputGroupKey(connection);
    const existing = groupIndexById.get(groupId);
    const groupIndex = typeof existing === "number" ? existing : groupIds.length;
    if (typeof existing !== "number") {
      groupIds.push(groupId);
      groupIndexById.set(groupId, groupIndex);
    }
    return groupIndex;
  });
  const adjacent = new Map<number, Set<number>>();
  const connectGroups = (leftGroupIndex: number, rightGroupIndex: number) => {
    if (leftGroupIndex === rightGroupIndex) return;
    if (!adjacent.has(leftGroupIndex)) adjacent.set(leftGroupIndex, new Set());
    if (!adjacent.has(rightGroupIndex)) adjacent.set(rightGroupIndex, new Set());
    adjacent.get(leftGroupIndex)?.add(rightGroupIndex);
    adjacent.get(rightGroupIndex)?.add(leftGroupIndex);
  };

  for (let left = 0; left < segments.length; left += 1) {
    for (let right = left + 1; right < segments.length; right += 1) {
      if (!routeSegmentsOverlap(segments[left], segments[right])) continue;
      const leftGroupIndex = groupIndexByConnectionIndex[segments[left].connectionIndex];
      const rightGroupIndex = groupIndexByConnectionIndex[segments[right].connectionIndex];
      connectGroups(leftGroupIndex, rightGroupIndex);
    }
  }

  const paletteSize = Math.max(1, Math.trunc(colorCount) || GRAPH_EDGE_COLOR_COUNT);
  const colors = new Map<number, number>();
  const ordered = [...adjacent.entries()]
    .sort((a, b) => b[1].size - a[1].size || a[0] - b[0])
    .map(([index]) => index);

  for (const groupIndex of ordered) {
    const used = new Set([...adjacent.get(groupIndex) ?? []]
      .map((neighborGroupIndex) => colors.get(neighborGroupIndex))
      .filter((colorIndex): colorIndex is number => typeof colorIndex === "number"));
    let colorIndex = 0;
    while (colorIndex < paletteSize && used.has(colorIndex)) {
      colorIndex += 1;
    }
    colors.set(groupIndex, colorIndex % paletteSize);
  }

  const result = new Map<string, number>();
  connections.forEach((connection, index) => {
    const colorIndex = colors.get(groupIndexByConnectionIndex[index]);
    if (typeof colorIndex === "number") {
      result.set(connectionKey(connection, index), colorIndex);
    }
  });
  return result;
}

export function colorGraphOverlappingRoutes(
  data: Pick<GraphData, "connections" | "links" | "edges">,
  colorCount = GRAPH_EDGE_COLOR_COUNT,
): GraphLink[] {
  const colorById = graphRouteColorIndexById(data, colorCount);
  return graphConnections(data).map((connection, index) => {
    const key = connectionKey(connection, index);
    const colorIndex = colorById.get(key);
    if (typeof colorIndex !== "number") {
      if (!connection.style?.hasOverlap && connection.style?.colorIndex === undefined && connection.style?.overlapGroupId === undefined) {
        return connection;
      }
      const nextStyle = { ...connection.style };
      delete nextStyle.colorIndex;
      delete nextStyle.hasOverlap;
      delete nextStyle.overlapGroupId;
      return {
        ...connection,
        style: Object.keys(nextStyle).length ? nextStyle : undefined,
      };
    }

    return {
      ...connection,
      style: {
        ...connection.style,
        colorIndex,
        hasOverlap: true,
        overlapGroupId: "route-overlap",
      },
    };
  });
}

export function graphHasMissingPositions(graph: GraphData): boolean {
  return graph.nodes.some((node) =>
    typeof node.x !== "number"
    || !Number.isFinite(node.x)
    || typeof node.y !== "number"
    || !Number.isFinite(node.y),
  );
}

export function clearGraphLinkRoutes(graph: GraphData): GraphData {
  const connections = graphConnections(graph).map((connection) => ({
    ...connection,
    points: undefined,
  }));
  return {
    ...graph,
    connections,
    links: connections,
  };
}
