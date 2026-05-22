import {
  computed,
  defineComponent,
  h,
  markRaw,
  nextTick,
  onBeforeUnmount,
  onBeforeUpdate,
  onMounted,
  reactive,
  ref,
  type PropType,
  type VNodeRef,
} from "vue";
import { GraphViewController, type GraphController } from "./graphController";
import { layoutGraphDocument } from "./graphLayout";
import { useLocusGraphStyles } from "./graphStyles";
import type {
  GraphAutoLayoutMode,
  GraphData,
  GraphEndpoint,
  GraphLayoutDirection,
  GraphLayoutOptions,
  GraphLink,
  GraphNode,
  GraphNodePortsConfig,
  GraphParameter,
  GraphPoint,
  GraphPort,
  GraphPortDirection,
} from "./graphTypes";
import {
  GRAPH_NODE_WIDTH,
  GRAPH_NODE_PORT_ID,
  GRAPH_WORLD_SIZE,
  cloneGraphData,
  graphConnections,
  graphHasMissingPositions,
  graphNodePortAnchor,
  graphRouteColorIndexById,
  graphRoutePointsWithAnchors,
  normalizeGraphData,
} from "./graphTypes";

const GRAPH_EDGE_CORNER_RADIUS = 12;
const GRAPH_EDGE_COLOR_COUNT = 6;

interface PendingGraphConnection {
  nodeId: string;
  portId: string | null;
  direction: GraphPortDirection;
}

function shouldIgnoreNodeDrag(target: EventTarget | null): boolean {
  return target instanceof Element
    && !!target.closest("input, select, textarea, button, .locus-graph-port");
}

function shouldAutoLayout(graph: GraphData, mode: GraphAutoLayoutMode, force = false): boolean {
  const graphMode = graph.layout?.auto ?? mode;
  if (force) return graphMode !== false && graphMode !== "off";
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
    setup(props) {
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
      const viewport = reactive({ x: 24, y: 24, scale: 1 });
      const viewportEl = ref<HTMLElement | null>(null);
      const nodeElements = new Map<string, HTMLElement>();
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

      function nodePortsConfig(): GraphNodePortsConfig {
        return graph.layout?.nodePorts ?? props.layoutOptions.nodePorts ?? true;
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
        const nextGraph = await layoutGraphDocument(currentGraph, props.layoutOptions);
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
            replaceGraphData(await layoutGraphDocument(normalized, props.layoutOptions));
          } else {
            replaceGraphData(normalized);
          }
          status.value = "Ready";
          await nextTick();
          fitGraph();
        } catch (loadError) {
          error.value = loadError instanceof Error ? loadError.message : String(loadError);
          status.value = "Error";
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
        } finally {
          loading.value = false;
        }
      }

      function fitGraph() {
        const container = viewportEl.value;
        if (!container || !graph.nodes.length) return;

        const bounds = graph.nodes.reduce(
          (result, node) => {
            const width = node.width ?? GRAPH_NODE_WIDTH;
            const height = nodeElements.get(node.id)?.offsetHeight ?? node.height ?? 112;
            return {
              minX: Math.min(result.minX, node.x ?? 0),
              minY: Math.min(result.minY, node.y ?? 0),
              maxX: Math.max(result.maxX, (node.x ?? 0) + width),
              maxY: Math.max(result.maxY, (node.y ?? 0) + height),
            };
          },
          { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity },
        );

        const width = Math.max(bounds.maxX - bounds.minX, 1);
        const height = Math.max(bounds.maxY - bounds.minY, 1);
        const nextScale = Math.min(
          1.25,
          Math.max(0.45, Math.min((container.clientWidth - 72) / width, (container.clientHeight - 72) / height)),
        );
        viewport.scale = Number.isFinite(nextScale) ? nextScale : 1;
        viewport.x = Math.round((container.clientWidth - width * viewport.scale) / 2 - bounds.minX * viewport.scale);
        viewport.y = Math.round((container.clientHeight - height * viewport.scale) / 2 - bounds.minY * viewport.scale);
        scheduleEdgeRender();
      }

      function nodeById(id: string) {
        return graph.nodes.find((node) => node.id === id) ?? null;
      }

      function endpointPoint(endpoint: GraphEndpoint, direction: GraphPortDirection) {
        const node = nodeById(endpoint.nodeId);
        if (!node) return null;
        return graphNodePortAnchor(node, direction, endpoint.portId);
      }

      function connectionBezierPath(start: GraphPoint, end: GraphPoint) {
        const dx = Math.max(56, Math.abs(end.x - start.x) * 0.5);
        return `M ${start.x} ${start.y} C ${start.x + dx} ${start.y}, ${end.x - dx} ${end.y}, ${end.x} ${end.y}`;
      }

      function connectionPath(connection: GraphLink) {
        edgeVersion.value;
        const start = endpointPoint(connection.from, "output");
        const end = endpointPoint(connection.to, "input");
        if (!start || !end) return "";
        if (draggingNodeId.value && connectionTouchesNode(connection, draggingNodeId.value)) {
          return connectionBezierPath(start, end);
        }
        if (connection.points && connection.points.length >= 2) {
          return graphPathWithEndpoints(connection.points, start, end, currentLayoutDirection());
        }
        return connectionBezierPath(start, end);
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

      function onGraphKeydown(event: KeyboardEvent) {
        if ((event.key === "Delete" || event.key === "Backspace") && (selectedConnectionId.value || selectedNodeId.value)) {
          event.preventDefault();
          removeSelectedItem();
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
        return h("button", {
          type: "button",
          onClick: addNodeFromController,
          disabled: props.readonly || loading.value,
        }, "Add");
      }

      function updateParameter(node: GraphNode, parameter: GraphParameter, value: unknown) {
        if (props.readonly || parameter.readOnly) return;
        parameter.value = value;
        selectedNodeId.value = node.id;
        notifyGraphChange();
      }

      function onNodePointerDown(event: PointerEvent, node: GraphNode) {
        if (shouldIgnoreNodeDrag(event.target)) return;
        event.preventDefault();
        event.stopPropagation();
        selectedNodeId.value = node.id;
        selectedConnectionId.value = "";
        const target = event.currentTarget as HTMLElement;
        const startX = event.clientX;
        const startY = event.clientY;
        const nodeStartX = node.x ?? 0;
        const nodeStartY = node.y ?? 0;
        let didDrag = false;
        target.setPointerCapture(event.pointerId);

        const onMove = (moveEvent: PointerEvent) => {
          if (!didDrag) {
            didDrag = true;
            draggingNodeId.value = node.id;
            clearConnectionRoutesForNode(node.id);
          }
          node.x = Math.round(nodeStartX + (moveEvent.clientX - startX) / viewport.scale);
          node.y = Math.round(nodeStartY + (moveEvent.clientY - startY) / viewport.scale);
          if (!props.readonly) dirty.value = true;
          scheduleEdgeRender();
        };
        const onUp = (upEvent: PointerEvent) => {
          try {
            target.releasePointerCapture(upEvent.pointerId);
          } catch {
            // Pointer capture may already be released by the host WebView.
          }
          target.removeEventListener("pointermove", onMove);
          target.removeEventListener("pointerup", onUp);
          target.removeEventListener("pointercancel", onUp);
          draggingNodeId.value = "";
          if (didDrag && !props.readonly) notifyGraphChange();
          if (didDrag) scheduleEdgeRender();
        };

        target.addEventListener("pointermove", onMove);
        target.addEventListener("pointerup", onUp);
        target.addEventListener("pointercancel", onUp);
      }

      function onViewportPointerDown(event: PointerEvent) {
        if (event.button !== 0 || shouldIgnoreNodeDrag(event.target)) return;
        const targetElement = event.target instanceof Element ? event.target : null;
        if (targetElement?.closest(".locus-graph-node")) return;
        const viewportNode = viewportEl.value;
        if (!viewportNode) return;
        event.preventDefault();
        selectedNodeId.value = "";
        selectedConnectionId.value = "";
        const startX = event.clientX;
        const startY = event.clientY;
        const viewportStartX = viewport.x;
        const viewportStartY = viewport.y;
        viewportNode.setPointerCapture(event.pointerId);

        const onMove = (moveEvent: PointerEvent) => {
          viewport.x = Math.round(viewportStartX + moveEvent.clientX - startX);
          viewport.y = Math.round(viewportStartY + moveEvent.clientY - startY);
          scheduleEdgeRender();
        };
        const onUp = (upEvent: PointerEvent) => {
          try {
            viewportNode.releasePointerCapture(upEvent.pointerId);
          } catch {
            // Pointer capture may already be released by the host WebView.
          }
          viewportNode.removeEventListener("pointermove", onMove);
          viewportNode.removeEventListener("pointerup", onUp);
          viewportNode.removeEventListener("pointercancel", onUp);
        };

        viewportNode.addEventListener("pointermove", onMove);
        viewportNode.addEventListener("pointerup", onUp);
        viewportNode.addEventListener("pointercancel", onUp);
      }

      function onWheel(event: WheelEvent) {
        const container = viewportEl.value;
        if (!container) return;
        event.preventDefault();
        const rect = container.getBoundingClientRect();
        const graphX = (event.clientX - rect.left - viewport.x) / viewport.scale;
        const graphY = (event.clientY - rect.top - viewport.y) / viewport.scale;
        const factor = event.deltaY < 0 ? 1.08 : 0.92;
        const nextScale = Math.min(1.8, Math.max(0.35, viewport.scale * factor));
        viewport.x = Math.round(event.clientX - rect.left - graphX * nextScale);
        viewport.y = Math.round(event.clientY - rect.top - graphY * nextScale);
        viewport.scale = nextScale;
        scheduleEdgeRender();
      }

      function renderPort(node: GraphNode, direction: GraphPortDirection, port?: GraphPort | null) {
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
        const commonProps = {
          disabled,
          "aria-label": label,
        };
        let control;
        if (type === "boolean") {
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

      function renderNode(node: GraphNode) {
        const parameters = node.parameters ?? [];
        const title = node.title || node.id;
        const subtitle = node.subtitle || node.type || "";
        const hasInputs = (node.inputs ?? []).length > 0;
        const hasOutputs = (node.outputs ?? []).length > 0;
        const hasNodeInputPort = shouldRenderNodePort("input");
        const hasNodeOutputPort = shouldRenderNodePort("output");
        const setNodeRef: VNodeRef = (element) => {
          if (element instanceof HTMLElement) nodeElements.set(node.id, element);
        };
        return h("div", {
          key: node.id,
          class: [
            "locus-graph-node",
            selectedNodeId.value === node.id ? "selected" : "",
            hasInputs ? "has-inputs" : "no-inputs",
            hasOutputs ? "has-outputs" : "no-outputs",
            !hasInputs && hasOutputs ? "output-only" : "",
            hasInputs && !hasOutputs ? "input-only" : "",
          ],
          style: {
            left: `${node.x ?? 0}px`,
            top: `${node.y ?? 0}px`,
            width: `${node.width ?? GRAPH_NODE_WIDTH}px`,
          },
          tabindex: 0,
          role: "button",
          ref: setNodeRef,
          onPointerdown: (event: PointerEvent) => onNodePointerDown(event, node),
          onClick: () => {
            selectedNodeId.value = node.id;
            selectedConnectionId.value = "";
          },
          onKeydown: (event: KeyboardEvent) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              selectedNodeId.value = node.id;
              selectedConnectionId.value = "";
            }
          },
        }, [
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
        ]);
      }

      function renderConnection(connection: GraphLink) {
        const selected = selectedConnectionId.value === connection.id;
        const colorIndex = connectionColorIndex(connection);
        return h("path", {
          key: connection.id,
          class: [
            "locus-graph-edge",
            `route-color-${colorIndex}`,
            connection.style?.hasOverlap || colorIndex > 0 ? "has-route-overlap" : "",
            selected ? "selected" : "",
          ],
          d: connectionPath(connection),
          onClick: (event: MouseEvent) => {
            event.stopPropagation();
            selectedConnectionId.value = connection.id || "";
            selectedNodeId.value = "";
          },
        });
      }

      onBeforeUpdate(() => {
        nodeElements.clear();
      });

      onMounted(() => {
        void loadGraph();
        window.addEventListener("resize", scheduleEdgeRender);
      });

      onBeforeUnmount(() => {
        if (edgeRenderFrame) window.cancelAnimationFrame(edgeRenderFrame);
        window.removeEventListener("resize", scheduleEdgeRender);
      });

      return () => h("section", { class: "locus-graph-view" }, [
        h("header", { class: "locus-graph-toolbar" }, [
          h("div", { class: "locus-graph-heading" }, [
            h("div", { class: "locus-graph-title" }, props.title),
            h("div", { class: "locus-graph-status" }, dirty.value ? `${status.value} · Modified` : status.value),
          ]),
          h("div", { class: "locus-graph-actions" }, [
            h("button", { type: "button", onClick: loadGraph, disabled: loading.value }, "Reload"),
            h("button", { type: "button", onClick: fitGraph, disabled: loading.value }, "Fit"),
            h("button", { type: "button", onClick: autoLayoutGraph, disabled: loading.value || !graph.nodes.length }, "Layout"),
            maybeRenderCreateNodeButton(),
            h("button", { type: "button", onClick: removeSelectedItem, disabled: props.readonly || !hasSelection() }, "Delete"),
            props.showPersistenceActions
              ? h("button", { type: "button", onClick: saveGraph, disabled: loading.value || props.readonly }, "Save")
              : null,
            props.showPersistenceActions
              ? h("button", { type: "button", onClick: applyGraph, disabled: loading.value || props.readonly }, "Apply")
              : null,
          ]),
        ]),
        error.value ? h("div", { class: "locus-graph-error" }, error.value) : null,
        h("div", {
          class: "locus-graph-viewport",
          ref: viewportEl,
          tabindex: 0,
          onPointerdown: onViewportPointerDown,
          onKeydown: onGraphKeydown,
          onWheel,
        }, [
          h("div", {
            class: "locus-graph-world",
            style: {
              width: `${GRAPH_WORLD_SIZE}px`,
              height: `${GRAPH_WORLD_SIZE}px`,
              transform: `translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.scale})`,
            },
          }, [
            h("svg", {
              class: "locus-graph-edge-layer",
              viewBox: `0 0 ${GRAPH_WORLD_SIZE} ${GRAPH_WORLD_SIZE}`,
              width: GRAPH_WORLD_SIZE,
              height: GRAPH_WORLD_SIZE,
              "aria-hidden": "true",
            }, graphConnections(graph).map(renderConnection)),
            graph.nodes.map(renderNode),
          ]),
        ]),
      ]);
    },
  });
}

export const LocusGraphView = markRaw(createGraphViewComponent());
