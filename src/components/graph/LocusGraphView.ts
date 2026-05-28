import {
  computed,
  defineComponent,
  h,
  markRaw,
  nextTick,
  onBeforeUnmount,
  onMounted,
  reactive,
  ref,
  type PropType,
} from "vue";
import { CanvasView, type CanvasItemMoveEvent, type CanvasViewExpose, type CanvasViewport } from "../canvas";
import BaseButton from "../ui/BaseButton.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import { GraphViewController, type GraphController } from "./graphController";
import { layoutGraphDocument } from "./graphLayout";
import { useLocusGraphStyles } from "./graphStyles";
import type {
  GraphAutoLayoutMode,
  GraphData,
  GraphEndpoint,
  GraphLayoutDirection,
  GraphLayoutMode,
  GraphLayoutOptions,
  GraphLink,
  GraphNode,
  GraphNodePortsConfig,
  GraphNodeStyle,
  GraphParameter,
  GraphPortSide,
  GraphPoint,
  GraphPort,
  GraphPortDirection,
  GraphStatePortPlacement,
} from "./graphTypes";
import {
  GRAPH_LAYOUT_MODES,
  GRAPH_NODE_PORT_ID,
  GRAPH_WORLD_SIZE,
  cloneGraphData,
  graphConnections,
  graphHasMissingPositions,
  graphNodePortAnchor,
  graphRouteColorIndexById,
  graphRoutePointsWithAnchors,
  graphStateNodePortSide,
  normalizeGraphLayoutMode,
  normalizeGraphNodeStyle,
  normalizeGraphStatePortPlacement,
  normalizeGraphData,
} from "./graphTypes";

const GRAPH_EDGE_CORNER_RADIUS = 12;
const GRAPH_EDGE_COLOR_COUNT = 6;
const GRAPH_LAYOUT_MODE_LABELS: Record<GraphLayoutMode, string> = {
  flow: "Flow",
  dependency: "Dependency",
  state: "State",
  radial: "Radial",
  manual: "Manual",
};
const GRAPH_LAYOUT_MODE_OPTIONS = GRAPH_LAYOUT_MODES.map((mode) => ({
  value: mode,
  label: GRAPH_LAYOUT_MODE_LABELS[mode],
}));

interface PendingGraphConnection {
  nodeId: string;
  portId: string | null;
  direction: GraphPortDirection;
}

interface GraphDirectionMarker {
  x: number;
  y: number;
  angle: number;
}

interface GraphFormulaToken {
  text: string;
  type: "identifier" | "number" | "operator" | "punctuation" | "string" | "text" | "space";
}

function shouldAutoLayout(graph: GraphData, mode: GraphAutoLayoutMode, force = false): boolean {
  const graphMode = graph.layout?.auto ?? mode;
  if (force) return true;
  if (graphMode === "always") return true;
  if (graphMode === false || graphMode === "off") return false;
  return graphHasMissingPositions(graph);
}

function endpointKey(nodeId: string, direction: GraphPortDirection, portId?: string | null): string {
  return `${direction}:${nodeId}:${portId || GRAPH_NODE_PORT_ID}`;
}

function graphCoord(value: number): string {
  if (Number.isInteger(value)) return String(value);
  return value.toFixed(2).replace(/\.?0+$/, "");
}

function graphDisplayValue(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function graphFormulaTokens(value: string): GraphFormulaToken[] {
  const tokens: GraphFormulaToken[] = [];
  const pattern = /(\s+|"(?:\\.|[^"])*"|'(?:\\.|[^'])*'|->|\u2192|[+\-*/=\u00d7\u00f7<>!&|]+|\d+(?:\.\d+)?|[()[\]{},.;:]|[A-Za-z_][A-Za-z0-9_]*|[^\s]+)/g;
  for (const match of value.matchAll(pattern)) {
    const text = match[0];
    let type: GraphFormulaToken["type"] = "text";
    if (/^\s+$/.test(text)) type = "space";
    else if (/^"(?:\\.|[^"])*"$|^'(?:\\.|[^'])*'$/.test(text)) type = "string";
    else if (/^\d+(?:\.\d+)?$/.test(text)) type = "number";
    else if (/^(?:->|\u2192|[+\-*/=\u00d7\u00f7<>!&|]+)$/.test(text)) type = "operator";
    else if (/^[()[\]{},.;:]$/.test(text)) type = "punctuation";
    else if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(text)) type = "identifier";
    tokens.push({ text, type });
  }
  return tokens;
}

function renderGraphFormulaCode(value: string) {
  return h("code", { class: "locus-graph-formula" }, graphFormulaTokens(value).map((token, index) => {
    if (token.type === "space") return token.text;
    return h("span", {
      key: `${index}:${token.type}`,
      class: ["locus-graph-formula-token", `token-${token.type}`],
    }, token.text);
  }));
}

function graphDistance(a: GraphPoint, b: GraphPoint): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

function compactGraphPoints(points: GraphPoint[]): GraphPoint[] {
  const result: GraphPoint[] = [];
  for (const point of points) {
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) continue;
    const previous = result[result.length - 1];
    if (previous && graphDistance(previous, point) < 0.5) continue;
    result.push(point);
  }
  return result;
}

function roundedGraphPathFromPoints(points: GraphPoint[], radius = GRAPH_EDGE_CORNER_RADIUS): string {
  const route = compactGraphPoints(points);
  if (route.length < 2) return "";

  let path = `M ${graphCoord(route[0].x)} ${graphCoord(route[0].y)}`;
  for (let index = 1; index < route.length - 1; index += 1) {
    const previous = route[index - 1];
    const current = route[index];
    const next = route[index + 1];
    const previousLength = graphDistance(previous, current);
    const nextLength = graphDistance(current, next);
    if (previousLength < 1 || nextLength < 1) {
      path += ` L ${graphCoord(current.x)} ${graphCoord(current.y)}`;
      continue;
    }
    const previousVector = {
      x: (current.x - previous.x) / previousLength,
      y: (current.y - previous.y) / previousLength,
    };
    const nextVector = {
      x: (next.x - current.x) / nextLength,
      y: (next.y - current.y) / nextLength,
    };
    const cross = previousVector.x * nextVector.y - previousVector.y * nextVector.x;
    if (Math.abs(cross) < 0.01) {
      path += ` L ${graphCoord(current.x)} ${graphCoord(current.y)}`;
      continue;
    }

    const cornerRadius = Math.min(radius, previousLength / 2, nextLength / 2);
    const cornerStart = {
      x: current.x - previousVector.x * cornerRadius,
      y: current.y - previousVector.y * cornerRadius,
    };
    const cornerEnd = {
      x: current.x + nextVector.x * cornerRadius,
      y: current.y + nextVector.y * cornerRadius,
    };
    path += ` L ${graphCoord(cornerStart.x)} ${graphCoord(cornerStart.y)}`;
    path += ` Q ${graphCoord(current.x)} ${graphCoord(current.y)} ${graphCoord(cornerEnd.x)} ${graphCoord(cornerEnd.y)}`;
  }

  const last = route[route.length - 1];
  return `${path} L ${graphCoord(last.x)} ${graphCoord(last.y)}`;
}

function graphPathWithEndpoints(
  points: GraphPoint[],
  start: GraphPoint | null,
  end: GraphPoint | null,
  direction: GraphLayoutDirection,
): string {
  if (!start || !end) return "";
  const next = graphRoutePointsWithAnchors(points, start, end, direction);
  return roundedGraphPathFromPoints(next);
}

function graphBezierControls(start: GraphPoint, end: GraphPoint, vertical = false) {
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

function graphCubicPoint(
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

function graphCubicTangent(
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

function graphDirectionMarkerFromBezier(
  start: GraphPoint,
  end: GraphPoint,
  vertical = false,
): GraphDirectionMarker | null {
  const { controlA, controlB } = graphBezierControls(start, end, vertical);
  const point = graphCubicPoint(start, controlA, controlB, end, 0.5);
  const tangent = graphCubicTangent(start, controlA, controlB, end, 0.5);
  if (Math.hypot(tangent.x, tangent.y) < 0.5) return null;
  return {
    x: point.x,
    y: point.y,
    angle: Math.atan2(tangent.y, tangent.x),
  };
}

function graphDirectionMarkerFromPoints(points: GraphPoint[]): GraphDirectionMarker | null {
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

function graphDirectionChevronPath(marker: GraphDirectionMarker, viewportScale: number): string {
  const scale = Number.isFinite(viewportScale)
    ? Math.min(2, Math.max(0.35, viewportScale))
    : 1;
  const length = 9 / scale;
  const spread = 4.5 / scale;
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
    "M", graphCoord(left.x), graphCoord(left.y),
    "L", graphCoord(apex.x), graphCoord(apex.y),
    "M", graphCoord(right.x), graphCoord(right.y),
    "L", graphCoord(apex.x), graphCoord(apex.y),
  ].join(" ");
}

function createGraphViewComponent() {
  return defineComponent({
    name: "LocusGraphView",
    props: {
      controller: {
        type: Object as PropType<GraphController>,
        default: () => new GraphViewController(),
      },
      title: {
        type: String,
        default: "Graph",
      },
      readonly: {
        type: Boolean,
        default: false,
      },
      autoLayout: {
        type: [Boolean, String] as PropType<GraphAutoLayoutMode>,
        default: "missing",
      },
      layoutOptions: {
        type: Object as PropType<GraphLayoutOptions>,
        default: () => ({}),
      },
      showPersistenceActions: {
        type: Boolean,
        default: true,
      },
    },
    setup(props, { slots }) {
      useLocusGraphStyles();

      const graph = reactive<GraphData>({
        schema: "locus.graph.v1",
        nodes: [],
        connections: [],
        links: [],
      });
      const status = ref("Ready");
      const error = ref("");
      const loading = ref(false);
      const dirty = ref(false);
      const selectedNodeId = ref("");
      const selectedConnectionId = ref("");
      const pendingConnection = ref<PendingGraphConnection | null>(null);
      const draggingNodeId = ref("");
      const edgeVersion = ref(0);
      const canvasRef = ref<CanvasViewExpose | null>(null);
      let edgeRenderScheduled = false;
      let edgeRenderFrame = 0;
      const routeColorIndexById = computed(() => {
        return graphRouteColorIndexById({ connections: graphConnections(graph) });
      });
      const portColorIndexByKey = computed(() => {
        const result = new Map<string, number>();
        for (const connection of graphConnections(graph)) {
          const colorIndex = connectionColorIndex(connection);
          const fromKey = endpointKey(connection.from.nodeId, "output", connection.from.portId);
          const toKey = endpointKey(connection.to.nodeId, "input", connection.to.portId);
          if (!result.has(fromKey)) result.set(fromKey, colorIndex);
          if (!result.has(toKey)) result.set(toKey, colorIndex);
        }
        return result;
      });

      function replaceGraphData(data: GraphData | null | undefined, options: { dirty?: boolean } = {}) {
        const normalized = normalizeGraphData(data);
        const nodes = normalized.nodes.map((node, index) => ({
          ...node,
          x: typeof node.x === "number" && Number.isFinite(node.x) ? node.x : 80 + index * 260,
          y: typeof node.y === "number" && Number.isFinite(node.y) ? node.y : 80 + (index % 3) * 140,
        }));
        const connections = graphConnections(normalized);
        graph.schema = normalized.schema;
        graph.layout = normalized.layout;
        graph.nodes.splice(0, graph.nodes.length, ...nodes);
        graph.connections = connections;
        graph.links = connections;
        dirty.value = !!options.dirty;
        pendingConnection.value = null;
        selectedConnectionId.value = "";
        selectedNodeId.value = graph.nodes[0]?.id ?? "";
        scheduleEdgeRender();
      }

      function snapshotGraph(): GraphData {
        return cloneGraphData({
          ...graph,
          nodes: graph.nodes,
          connections: graphConnections(graph),
        });
      }

      function scheduleEdgeRender() {
        if (edgeRenderScheduled) return;
        edgeRenderScheduled = true;
        const flush = () => {
          edgeRenderScheduled = false;
          edgeRenderFrame = 0;
          edgeVersion.value += 1;
        };
        if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
          edgeRenderFrame = window.requestAnimationFrame(flush);
        } else {
          void nextTick(flush);
        }
      }

      function currentLayoutDirection(): GraphLayoutDirection {
        return graph.layout?.direction ?? props.layoutOptions.direction ?? "right";
      }

      function currentStatePortPlacement(): GraphStatePortPlacement {
        return normalizeGraphStatePortPlacement(
          graph.layout?.statePortPlacement ?? props.layoutOptions.statePortPlacement,
        );
      }

      function currentLayoutMode(): GraphLayoutMode {
        return normalizeGraphLayoutMode(graph.layout?.mode ?? props.layoutOptions.mode);
      }

      function effectiveLayoutOptions(overrides: GraphLayoutOptions = {}): GraphLayoutOptions {
        return {
          ...props.layoutOptions,
          ...graph.layout,
          ...overrides,
        };
      }

      function nodePortsConfig(): GraphNodePortsConfig {
        return graph.layout?.nodePorts ?? props.layoutOptions.nodePorts ?? true;
      }

      function effectiveNodeStyle(node: GraphNode): GraphNodeStyle {
        return normalizeGraphNodeStyle(node.nodeStyle ?? graph.layout?.nodeStyle ?? props.layoutOptions.nodeStyle);
      }

      function graphUsesDirectedConnections(): boolean {
        return graph.layout?.directed ?? props.layoutOptions.directed ?? false;
      }

      function connectionIsDirected(connection: GraphLink): boolean {
        return connection.directed ?? graphUsesDirectedConnections();
      }

      function shouldRenderNodePort(direction: GraphPortDirection) {
        const config = nodePortsConfig();
        if (typeof config === "boolean") return config;
        return !!config[direction];
      }

      function connectionTouchesNode(connection: GraphLink, nodeId: string) {
        return connection.from.nodeId === nodeId || connection.to.nodeId === nodeId;
      }

      function withoutRouteOverlapStyle(style: GraphLink["style"]) {
        if (!style) return undefined;
        const nextStyle = { ...style };
        delete nextStyle.colorIndex;
        delete nextStyle.hasOverlap;
        delete nextStyle.overlapGroupId;
        return Object.keys(nextStyle).length ? nextStyle : undefined;
      }

      function clearConnectionRoutesForNode(nodeId: string) {
        const connections = graphConnections(graph).map((connection) => {
          if (!connectionTouchesNode(connection, nodeId)) return connection;
          if (!connection.points && !connection.style?.hasOverlap && connection.style?.colorIndex === undefined) {
            return connection;
          }
          return {
            ...connection,
            points: undefined,
            style: withoutRouteOverlapStyle(connection.style),
          };
        });
        graph.connections = connections;
        graph.links = connections;
      }

      function notifyGraphChange() {
        dirty.value = true;
        scheduleEdgeRender();
        props.controller.onGraphChange?.(snapshotGraph());
      }

      async function layoutCurrentGraph(force = false, markDirty = false) {
        const currentGraph = snapshotGraph();
        if (!shouldAutoLayout(currentGraph, props.autoLayout, force)) return false;
        status.value = "Layout";
        const nextGraph = await layoutGraphDocument(currentGraph, effectiveLayoutOptions());
        replaceGraphData(nextGraph, { dirty: markDirty });
        if (markDirty) props.controller.onGraphChange?.(snapshotGraph());
        return true;
      }

      async function loadGraph() {
        loading.value = true;
        error.value = "";
        status.value = "Loading";
        try {
          const nextGraph = props.controller.loadGraph
            ? await props.controller.loadGraph()
            : { schema: "locus.graph.v1", nodes: [], connections: [] };
          const normalized = normalizeGraphData(nextGraph);
          if (shouldAutoLayout(normalized, props.autoLayout)) {
            replaceGraphData(await layoutGraphDocument(normalized, {
              ...props.layoutOptions,
              ...normalized.layout,
            }));
          } else {
            replaceGraphData(normalized);
          }
          status.value = "Ready";
          await nextTick();
          fitGraph();
        } catch (loadError) {
          error.value = loadError instanceof Error ? loadError.message : String(loadError);
          status.value = "Error";
          console.error("[GraphView] loadGraph failed", loadError);
        } finally {
          loading.value = false;
        }
      }

      async function saveGraph() {
        error.value = "";
        status.value = "Saving";
        try {
          await props.controller.saveGraph?.(snapshotGraph());
          dirty.value = false;
          status.value = "Saved";
        } catch (saveError) {
          error.value = saveError instanceof Error ? saveError.message : String(saveError);
          status.value = "Error";
          console.error("[GraphView] saveGraph failed", saveError);
        }
      }

      async function applyGraph() {
        error.value = "";
        status.value = "Applying";
        try {
          await props.controller.applyGraph?.(snapshotGraph());
          dirty.value = false;
          status.value = "Applied";
        } catch (applyError) {
          error.value = applyError instanceof Error ? applyError.message : String(applyError);
          status.value = "Error";
          console.error("[GraphView] applyGraph failed", applyError);
        }
      }

      async function autoLayoutGraph() {
        if (loading.value) return;
        loading.value = true;
        error.value = "";
        try {
          await layoutCurrentGraph(true, true);
          status.value = "Layout ready";
          await nextTick();
          fitGraph();
        } catch (layoutError) {
          error.value = layoutError instanceof Error ? layoutError.message : String(layoutError);
          status.value = "Error";
          console.error("[GraphView] layout failed", layoutError);
        } finally {
          loading.value = false;
        }
      }

      async function selectLayoutMode(modeValue: string) {
        const mode = normalizeGraphLayoutMode(modeValue);
        graph.layout = effectiveLayoutOptions({ mode });
        if (mode === "manual") {
          notifyGraphChange();
          status.value = "Manual";
          return;
        }
        if (!graph.nodes.length) {
          notifyGraphChange();
          return;
        }
        await autoLayoutGraph();
      }

      function fitGraph() {
        canvasRef.value?.fitContent();
      }

      function nodeById(id: string) {
        return graph.nodes.find((node) => node.id === id) ?? null;
      }

      function endpointPoint(endpoint: GraphEndpoint, direction: GraphPortDirection) {
        const node = nodeById(endpoint.nodeId);
        if (!node) return null;
        return graphNodePortAnchor(
          node,
          direction,
          endpoint.portId,
          currentLayoutDirection(),
          currentStatePortPlacement(),
        );
      }

      function portSideForEndpoint(endpoint: GraphEndpoint, direction: GraphPortDirection): GraphPortSide {
        const node = nodeById(endpoint.nodeId);
        if (node && effectiveNodeStyle(node) === "state") {
          return graphStateNodePortSide(direction, currentLayoutDirection(), currentStatePortPlacement());
        }
        return direction === "input" ? "left" : "right";
      }

      function connectionUsesVerticalPorts(connection: GraphLink): boolean {
        const fromSide = portSideForEndpoint(connection.from, "output");
        const toSide = portSideForEndpoint(connection.to, "input");
        return fromSide === "top" || fromSide === "bottom" || toSide === "top" || toSide === "bottom";
      }

      function connectionBezierPath(start: GraphPoint, end: GraphPoint, vertical = false) {
        const { controlA, controlB } = graphBezierControls(start, end, vertical);
        return [
          "M", start.x, start.y,
          "C", controlA.x, controlA.y,
          controlB.x, controlB.y,
          end.x, end.y,
        ].join(" ");
      }

      function connectionPath(connection: GraphLink) {
        edgeVersion.value;
        const start = endpointPoint(connection.from, "output");
        const end = endpointPoint(connection.to, "input");
        if (!start || !end) return "";
        if (draggingNodeId.value && connectionTouchesNode(connection, draggingNodeId.value)) {
          return connectionBezierPath(start, end, connectionUsesVerticalPorts(connection));
        }
        if (connection.points && connection.points.length >= 2) {
          return graphPathWithEndpoints(connection.points, start, end, currentLayoutDirection());
        }
        return connectionBezierPath(start, end, connectionUsesVerticalPorts(connection));
      }

      function connectionDirectionPath(connection: GraphLink, viewportScale: number) {
        edgeVersion.value;
        const start = endpointPoint(connection.from, "output");
        const end = endpointPoint(connection.to, "input");
        if (!start || !end) return "";

        const marker = !draggingNodeId.value && connection.points && connection.points.length >= 2
          ? graphDirectionMarkerFromPoints(graphRoutePointsWithAnchors(connection.points, start, end, currentLayoutDirection()))
          : graphDirectionMarkerFromBezier(start, end, connectionUsesVerticalPorts(connection));
        return marker ? graphDirectionChevronPath(marker, viewportScale) : "";
      }

      function connectionColorIndex(connection: GraphLink) {
        const styleColorIndex = connection.style?.colorIndex;
        const routeColorIndex = connection.id ? routeColorIndexById.value.get(connection.id) : undefined;
        const colorIndex = typeof routeColorIndex === "number" ? routeColorIndex : styleColorIndex;
        if (typeof colorIndex !== "number" || !Number.isFinite(colorIndex)) return 0;
        return Math.abs(Math.trunc(colorIndex)) % GRAPH_EDGE_COLOR_COUNT;
      }

      function portColorIndex(node: GraphNode, direction: GraphPortDirection, port?: GraphPort | null) {
        return portColorIndexByKey.value.get(endpointKey(node.id, direction, port?.id ?? null));
      }

      function beginConnection(node: GraphNode, direction: GraphPortDirection, port?: GraphPort | null) {
        if (props.readonly) return;
        const next: PendingGraphConnection = {
          nodeId: node.id,
          portId: port?.id ?? null,
          direction,
        };
        const pending = pendingConnection.value;
        if (!pending) {
          pendingConnection.value = next;
          status.value = direction === "output" ? "Select input" : "Select output";
          return;
        }
        if (pending.direction === direction) {
          pendingConnection.value = next;
          return;
        }

        const from = pending.direction === "output" ? pending : next;
        const to = pending.direction === "input" ? pending : next;
        const connection: GraphLink = {
          id: `connection-${Date.now().toString(36)}`,
          from: { nodeId: from.nodeId, portId: from.portId },
          to: { nodeId: to.nodeId, portId: to.portId },
        };
        const validation = props.controller.validateConnection?.(connection, snapshotGraph()) ?? true;
        const ok = typeof validation === "object" ? validation.ok : validation === true;
        if (!ok) {
          error.value = typeof validation === "string"
            ? validation
            : typeof validation === "object"
              ? validation.message || "Connection rejected."
              : "Connection rejected.";
          status.value = "Rejected";
          pendingConnection.value = null;
          return;
        }

        const connections = graphConnections(graph).filter((item) => {
          return !(
            item.to.nodeId === connection.to.nodeId
            && (item.to.portId ?? null) === (connection.to.portId ?? null)
            && item.from.nodeId === connection.from.nodeId
            && (item.from.portId ?? null) === (connection.from.portId ?? null)
          );
        });
        connections.push(connection);
        graph.connections = connections;
        graph.links = connections;
        selectedConnectionId.value = connection.id || "";
        pendingConnection.value = null;
        status.value = "Connected";
        notifyGraphChange();
      }

      function removeSelectedItem() {
        if (props.readonly) return;
        if (selectedConnectionId.value) {
          const connections = graphConnections(graph);
          const index = connections.findIndex((connection) => connection.id === selectedConnectionId.value);
          if (index >= 0) {
            connections.splice(index, 1);
            graph.connections = connections;
            graph.links = connections;
            selectedConnectionId.value = "";
            notifyGraphChange();
          }
          return;
        }
        if (selectedNodeId.value) {
          const nodeId = selectedNodeId.value;
          const index = graph.nodes.findIndex((node) => node.id === nodeId);
          if (index >= 0) {
            graph.nodes.splice(index, 1);
            const connections = graphConnections(graph).filter((connection) =>
              connection.from.nodeId !== nodeId && connection.to.nodeId !== nodeId,
            );
            graph.connections = connections;
            graph.links = connections;
            selectedNodeId.value = "";
            pendingConnection.value = null;
            notifyGraphChange();
          }
        }
      }

      function addNodeFromController() {
        const nextNode = props.controller.createNode?.(snapshotGraph());
        if (!nextNode || props.readonly) return;
        const normalized = normalizeGraphData({ nodes: [nextNode], connections: [] }).nodes[0];
        if (!normalized) return;
        if (graph.nodes.some((node) => node.id === normalized.id)) {
          error.value = `Node id already exists: ${normalized.id}`;
          return;
        }
        graph.nodes.push({
          ...normalized,
          x: typeof normalized.x === "number" ? normalized.x : 80 + graph.nodes.length * 260,
          y: typeof normalized.y === "number" ? normalized.y : 80 + (graph.nodes.length % 3) * 140,
        });
        selectedNodeId.value = normalized.id;
        notifyGraphChange();
      }

      function canCreateNode() {
        return typeof props.controller.createNode === "function";
      }

      function hasSelection() {
        return !!selectedConnectionId.value || !!selectedNodeId.value;
      }

      function maybeRenderCreateNodeButton() {
        if (!canCreateNode()) return null;
        return h(BaseButton, {
          size: "sm",
          variant: "neutral",
          onClick: addNodeFromController,
          disabled: props.readonly || loading.value,
        }, () => "Add");
      }

      function updateParameter(node: GraphNode, parameter: GraphParameter, value: unknown) {
        if (props.readonly || parameter.readOnly) return;
        parameter.value = value;
        selectedNodeId.value = node.id;
        notifyGraphChange();
      }

      function onCanvasSelectionUpdate(itemId: string) {
        selectedNodeId.value = itemId;
        selectedConnectionId.value = "";
      }

      function onCanvasItemDragStart(event: CanvasItemMoveEvent) {
        draggingNodeId.value = event.item.id;
        clearConnectionRoutesForNode(event.item.id);
      }

      function onCanvasItemMove() {
        if (!props.readonly) dirty.value = true;
        scheduleEdgeRender();
      }

      function onCanvasItemMoveEnd(event: CanvasItemMoveEvent) {
        draggingNodeId.value = "";
        if (event.didDrag && !props.readonly) notifyGraphChange();
        if (event.didDrag) scheduleEdgeRender();
      }

      function renderPort(
        node: GraphNode,
        direction: GraphPortDirection,
        port?: GraphPort | null,
        side?: GraphPortSide,
      ) {
        const portId = port?.id ?? null;
        const pending = pendingConnection.value;
        const active = !!pending
          && pending.nodeId === node.id
          && pending.direction === direction
          && (pending.portId ?? null) === portId;
        const colorIndex = portColorIndex(node, direction, port);
        const connected = typeof colorIndex === "number";
        return h("button", {
          type: "button",
          class: [
            "locus-graph-port",
            `locus-graph-port-${direction}`,
            side ? `locus-graph-port-side-${side}` : "",
            connected ? "connected" : "",
            connected ? `route-color-${colorIndex}` : "",
            active ? "active" : "",
          ],
          title: port?.label || (direction === "input" ? "Input" : "Output"),
          onPointerdown: (event: PointerEvent) => event.stopPropagation(),
          onClick: (event: MouseEvent) => {
            event.stopPropagation();
            beginConnection(node, direction, port);
          },
        });
      }

      function renderParameter(node: GraphNode, parameter: GraphParameter) {
        const type = parameter.type || "string";
        const disabled = props.readonly || !!parameter.readOnly;
        const label = parameter.label || parameter.id;
        const displayValue = graphDisplayValue(parameter.value);
        const commonProps = {
          disabled,
          "aria-label": label,
        };
        let control;
        if (disabled && type !== "boolean" && type !== "color") {
          control = h("span", {
            class: "locus-graph-parameter-value",
            title: displayValue,
          }, renderGraphFormulaCode(displayValue));
        } else if (type === "boolean") {
          control = h("input", {
            ...commonProps,
            type: "checkbox",
            checked: !!parameter.value,
            onChange: (event: Event) => {
              updateParameter(node, parameter, (event.target as HTMLInputElement).checked);
            },
          });
        } else if (type === "select") {
          control = h("select", {
            ...commonProps,
            value: String(parameter.value ?? ""),
            onChange: (event: Event) => {
              updateParameter(node, parameter, (event.target as HTMLSelectElement).value);
            },
          }, (parameter.options ?? []).map((option) =>
            h("option", { value: String(option.value) }, option.label),
          ));
        } else if (type === "text") {
          control = h("textarea", {
            ...commonProps,
            value: String(parameter.value ?? ""),
            placeholder: parameter.placeholder || "",
            onInput: (event: Event) => {
              updateParameter(node, parameter, (event.target as HTMLTextAreaElement).value);
            },
          });
        } else {
          control = h("input", {
            ...commonProps,
            type: type === "number" ? "number" : type === "color" ? "color" : "text",
            value: parameter.value ?? "",
            min: parameter.min,
            max: parameter.max,
            step: parameter.step,
            placeholder: parameter.placeholder || "",
            onInput: (event: Event) => {
              const target = event.target as HTMLInputElement;
              updateParameter(node, parameter, type === "number" ? Number(target.value) : target.value);
            },
          });
        }

        return h("label", { class: "locus-graph-parameter", key: parameter.id }, [
          h("span", { class: "locus-graph-parameter-label" }, label),
          control,
        ]);
      }

      function renderPortRows(node: GraphNode, direction: GraphPortDirection) {
        const ports = direction === "input" ? node.inputs ?? [] : node.outputs ?? [];
        if (!ports.length) {
          return h("div", {
            class: [
              "locus-graph-port-list",
              `locus-graph-port-list-${direction}`,
              "empty",
            ],
            "aria-hidden": "true",
          });
        }
        return h("div", { class: ["locus-graph-port-list", `locus-graph-port-list-${direction}`] },
          ports.map((port) =>
            h("div", { class: "locus-graph-port-row", key: port.id }, direction === "input"
              ? [
                  renderPort(node, "input", port),
                  h("span", { class: "locus-graph-port-label" }, port.label || port.id),
                ]
              : [
                  h("span", { class: "locus-graph-port-label" }, port.label || port.id),
                  renderPort(node, "output", port),
                ]),
          ),
        );
      }

      function graphNodeClass(item: GraphNode) {
        const hasInputs = (item.inputs ?? []).length > 0;
        const hasOutputs = (item.outputs ?? []).length > 0;
        const nodeStyle = effectiveNodeStyle(item);
        return [
          "locus-graph-node",
          `node-style-${nodeStyle}`,
          hasInputs ? "has-inputs" : "no-inputs",
          hasOutputs ? "has-outputs" : "no-outputs",
          !hasInputs && hasOutputs ? "output-only" : "",
          hasInputs && !hasOutputs ? "input-only" : "",
        ];
      }

      function renderStateNode(node: GraphNode) {
        const title = node.title || node.id;
        const subtitle = node.subtitle || node.type || "";
        const hasNodeInputPort = shouldRenderNodePort("input");
        const hasNodeOutputPort = shouldRenderNodePort("output");
        const inputSide = graphStateNodePortSide("input", currentLayoutDirection(), currentStatePortPlacement());
        const outputSide = graphStateNodePortSide("output", currentLayoutDirection(), currentStatePortPlacement());
        return h("div", { class: "locus-graph-state-node" }, [
          hasNodeInputPort ? renderPort(node, "input", null, inputSide) : null,
          h("div", { class: "locus-graph-state-node-main" }, [
            h("div", { class: "locus-graph-state-node-title" }, title),
            subtitle ? h("div", { class: "locus-graph-state-node-subtitle" }, subtitle) : null,
          ]),
          hasNodeOutputPort ? renderPort(node, "output", null, outputSide) : null,
        ]);
      }

      function renderNode(node: GraphNode) {
        const customNode = slots.node?.({
          node,
          selected: selectedNodeId.value === node.id,
          readonly: props.readonly,
          updateParameter: (parameter: GraphParameter, value: unknown) => updateParameter(node, parameter, value),
        });
        if (customNode?.length) return customNode;
        if (effectiveNodeStyle(node) === "state") return renderStateNode(node);

        const parameters = node.parameters ?? [];
        const title = node.title || node.id;
        const subtitle = node.subtitle || node.type || "";
        const hasInputs = (node.inputs ?? []).length > 0;
        const hasOutputs = (node.outputs ?? []).length > 0;
        const hasNodeInputPort = shouldRenderNodePort("input");
        const hasNodeOutputPort = shouldRenderNodePort("output");
        return [
          h("div", {
            class: [
              "locus-graph-node-header",
              hasNodeInputPort ? "node-port-input" : "",
              hasNodeOutputPort ? "node-port-output" : "",
            ],
          }, [
            hasNodeInputPort ? renderPort(node, "input", null) : null,
            h("div", { class: "locus-graph-node-title-block" }, [
              h("div", { class: "locus-graph-node-title" }, title),
              subtitle ? h("div", { class: "locus-graph-node-subtitle" }, subtitle) : null,
            ]),
            hasNodeOutputPort ? renderPort(node, "output", null) : null,
          ]),
          h("div", { class: "locus-graph-node-body" }, [
            h("div", { class: "locus-graph-port-groups" }, [
              renderPortRows(node, "input"),
              renderPortRows(node, "output"),
            ]),
            parameters.length
              ? h("div", {
                  class: [
                    "locus-graph-parameters",
                    !hasInputs && hasOutputs ? "align-output" : "",
                    hasInputs && !hasOutputs ? "align-input" : "",
                  ],
                }, parameters.map((parameter) => renderParameter(node, parameter)))
              : null,
          ]),
        ];
      }

      function renderConnection(connection: GraphLink, viewportScale: number) {
        const selected = selectedConnectionId.value === connection.id;
        const colorIndex = connectionColorIndex(connection);
        const key = connection.id || `${connection.from.nodeId}:${connection.from.portId ?? ""}:${connection.to.nodeId}:${connection.to.portId ?? ""}`;
        const edge = h("path", {
          key,
          class: [
            "locus-graph-edge",
            `route-color-${colorIndex}`,
            connection.style?.hasOverlap || colorIndex > 0 ? "has-route-overlap" : "",
            selected ? "selected" : "",
          ],
          d: connectionPath(connection),
          onPointerdown: (event: PointerEvent) => event.stopPropagation(),
          onClick: (event: MouseEvent) => {
            event.stopPropagation();
            selectedConnectionId.value = connection.id || "";
            selectedNodeId.value = "";
          },
        });
        const directionPath = connectionIsDirected(connection) ? connectionDirectionPath(connection, viewportScale) : "";
        if (!directionPath) return [edge];
        return [
          edge,
          h("path", {
            key: `${key}:direction`,
            class: [
              "locus-graph-edge-direction",
              `route-color-${colorIndex}`,
              selected ? "selected" : "",
            ],
            d: directionPath,
          }),
        ];
      }

      onMounted(() => {
        void loadGraph();
        window.addEventListener("resize", scheduleEdgeRender);
      });

      onBeforeUnmount(() => {
        if (edgeRenderFrame) window.cancelAnimationFrame(edgeRenderFrame);
        window.removeEventListener("resize", scheduleEdgeRender);
      });

      function renderToolbarButton(
        label: string,
        onClick: () => void,
        disabled = false,
        variant: "neutral" | "danger" = "neutral",
      ) {
        return h(BaseButton, {
          size: "sm",
          variant,
          disabled,
          onClick,
        }, () => label);
      }

      return () => h("section", { class: "locus-graph-view" }, [
        h("header", { class: "locus-graph-toolbar" }, [
          h("div", { class: "locus-graph-heading" }, [
            h("div", { class: "locus-graph-title" }, props.title),
            h("div", { class: "locus-graph-status" }, dirty.value ? `${status.value} · Modified` : status.value),
          ]),
          h("div", { class: "locus-graph-actions" }, [
            renderToolbarButton("Reload", loadGraph, loading.value),
            renderToolbarButton("Fit", fitGraph, loading.value),
            h(BaseDropdown, {
              class: "locus-graph-layout-mode",
              modelValue: currentLayoutMode(),
              selectedLabel: GRAPH_LAYOUT_MODE_LABELS[currentLayoutMode()],
              options: GRAPH_LAYOUT_MODE_OPTIONS,
              ariaLabel: "Layout mode",
              size: "sm",
              menuAlign: "end",
              disabled: loading.value || !graph.nodes.length,
              "onUpdate:modelValue": selectLayoutMode,
            }),
            renderToolbarButton("Layout", autoLayoutGraph, loading.value || !graph.nodes.length),
            maybeRenderCreateNodeButton(),
            renderToolbarButton("Delete", removeSelectedItem, props.readonly || !hasSelection(), "danger"),
            props.showPersistenceActions
              ? renderToolbarButton("Save", saveGraph, loading.value || props.readonly)
              : null,
            props.showPersistenceActions
              ? renderToolbarButton("Apply", applyGraph, loading.value || props.readonly)
              : null,
          ]),
        ]),
        error.value ? h("div", { class: "locus-graph-error" }, error.value) : null,
        h(CanvasView, {
          ref: canvasRef,
          items: graph.nodes,
          selectedItemId: selectedNodeId.value,
          selectionActive: hasSelection(),
          readonly: props.readonly,
          moveReadonly: true,
          worldSize: GRAPH_WORLD_SIZE,
          itemClass: (item: unknown) => graphNodeClass(item as GraphNode),
          ignoreDragSelector: "input, select, textarea, button, .locus-graph-port",
          "onUpdate:selectedItemId": onCanvasSelectionUpdate,
          onItemDragStart: onCanvasItemDragStart,
          onItemMove: onCanvasItemMove,
          onItemMoveEnd: onCanvasItemMoveEnd,
          onDeleteSelection: removeSelectedItem,
          onRender: scheduleEdgeRender,
        }, {
          overlay: ({ viewport }: { viewport: CanvasViewport }) =>
            h("svg", {
              class: "locus-graph-edge-layer",
              viewBox: `0 0 ${GRAPH_WORLD_SIZE} ${GRAPH_WORLD_SIZE}`,
              width: GRAPH_WORLD_SIZE,
              height: GRAPH_WORLD_SIZE,
              "aria-hidden": "true",
            }, graphConnections(graph).flatMap((connection) => renderConnection(connection, viewport.scale))),
          default: ({ item }: { item: unknown }) => renderNode(item as GraphNode),
        }),
      ]);
    },
  });
}

export const LocusGraphView = markRaw(createGraphViewComponent());
