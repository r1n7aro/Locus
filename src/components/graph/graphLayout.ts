import type { ElkExtendedEdge, ElkNode, ElkPort } from "elkjs";
import type {
  GraphData,
  GraphLayoutDirection,
  GraphLayoutMode,
  GraphLayoutOptions,
  GraphLink,
  GraphNode,
  GraphPort,
} from "./graphTypes";
import {
  GRAPH_NODE_BODY_PADDING_TOP,
  GRAPH_NODE_HEADER_HEIGHT,
  GRAPH_NODE_PORT_ID,
  GRAPH_PORT_ROW_PITCH,
  GRAPH_PORT_SIZE,
  colorGraphOverlappingRoutes,
  estimateGraphNodeHeightForStyle,
  estimateGraphNodeWidthForStyle,
  graphConnections,
  graphIsDag,
  graphNodePortAnchor,
  normalizeGraphStatePortPlacement,
  normalizeGraphLayoutMode,
  normalizeGraphNodeStyle,
  normalizeGraphData,
} from "./graphTypes";

type ElkConstructor = typeof import("elkjs").default;

const ELK_DIRECTION: Record<GraphLayoutDirection, string> = {
  right: "RIGHT",
  left: "LEFT",
  down: "DOWN",
  up: "UP",
};

const ELK_LAYOUT_ALGORITHM: Record<GraphLayoutMode, string> = {
  flow: "layered",
  dependency: "stress",
  state: "force",
  radial: "radial",
  manual: "layered",
};

let elkInstancePromise: Promise<InstanceType<ElkConstructor>> | null = null;

async function getElk() {
  if (!elkInstancePromise) {
    elkInstancePromise = import("elkjs/lib/elk.bundled.js").then((module) => {
      const Elk = module.default as ElkConstructor;
      return new Elk({
        defaultLayoutOptions: {
          "elk.algorithm": "layered",
        },
      });
    });
  }
  return elkInstancePromise;
}

function portShapeId(nodeId: string, portId?: string | null) {
  return `${nodeId}:${portId || GRAPH_NODE_PORT_ID}`;
}

function portSide(direction: "input" | "output", layoutDirection: GraphLayoutDirection): "WEST" | "EAST" | "NORTH" | "SOUTH" {
  if (layoutDirection === "down") return direction === "input" ? "NORTH" : "SOUTH";
  if (layoutDirection === "up") return direction === "input" ? "SOUTH" : "NORTH";
  if (layoutDirection === "left") return direction === "input" ? "EAST" : "WEST";
  return direction === "input" ? "WEST" : "EAST";
}

function elkPorts(node: GraphNode, layoutDirection: GraphLayoutDirection, nodeWidth: number): ElkPort[] {
  const inputs = (node.inputs ?? []).map((port, index) => elkPort(node.id, port, "input", index, layoutDirection, nodeWidth));
  const outputs = (node.outputs ?? []).map((port, index) => elkPort(node.id, port, "output", index, layoutDirection, nodeWidth));
  return [...inputs, ...outputs];
}

function elkPort(
  nodeId: string,
  port: GraphPort,
  direction: "input" | "output",
  index: number,
  layoutDirection: GraphLayoutDirection,
  nodeWidth: number,
): ElkPort {
  const centerY = GRAPH_NODE_HEADER_HEIGHT
    + GRAPH_NODE_BODY_PADDING_TOP
    + GRAPH_PORT_SIZE / 2
    + index * GRAPH_PORT_ROW_PITCH;
  const centerX = direction === "input" ? 0 : nodeWidth;

  return {
    id: portShapeId(nodeId, port.id),
    x: Math.round(centerX - GRAPH_PORT_SIZE / 2),
    y: Math.round(centerY - GRAPH_PORT_SIZE / 2),
    width: GRAPH_PORT_SIZE,
    height: GRAPH_PORT_SIZE,
    layoutOptions: {
      "elk.port.side": portSide(direction, layoutDirection),
      "elk.port.index": String(index),
    },
  };
}

function toElkNode(node: GraphNode, layoutDirection: GraphLayoutDirection, usePorts: boolean): ElkNode {
  const width = node.width ?? estimateGraphNodeWidthForStyle(node);
  const height = node.height ?? estimateGraphNodeHeightForStyle(node);
  return {
    id: node.id,
    width,
    height,
    ports: usePorts ? elkPorts(node, layoutDirection, width) : undefined,
    layoutOptions: {
      ...(usePorts ? { "elk.portConstraints": "FIXED_POS" } : {}),
      "elk.nodeSize.constraints": "MINIMUM_SIZE",
      "elk.nodeSize.minimum": `(${width},${height})`,
    },
  };
}

function endpointShapeId(graph: GraphData, endpoint: GraphLink["from"], direction: "input" | "output") {
  const node = graph.nodes.find((item) => item.id === endpoint.nodeId);
  if (!node || !endpoint.portId) return endpoint.nodeId;
  const ports = direction === "input" ? node.inputs ?? [] : node.outputs ?? [];
  const hasPort = ports.some((port) => port.id === endpoint.portId);
  return hasPort ? portShapeId(endpoint.nodeId, endpoint.portId) : endpoint.nodeId;
}

function toElkEdge(graph: GraphData, connection: GraphLink, index: number, usePorts: boolean): ElkExtendedEdge {
  return {
    id: connection.id || `edge-${index + 1}`,
    sources: [usePorts ? endpointShapeId(graph, connection.from, "output") : connection.from.nodeId],
    targets: [usePorts ? endpointShapeId(graph, connection.to, "input") : connection.to.nodeId],
    labels: connection.label ? [{ text: connection.label }] : undefined,
  };
}

function edgePoints(edge: ElkExtendedEdge | undefined) {
  if (!edge) return undefined;
  const section = edge.sections?.[0];
  if (!section) return undefined;
  return [
    section.startPoint,
    ...(section.bendPoints ?? []),
    section.endPoint,
  ].map((point) => ({
    x: Math.round(point.x),
    y: Math.round(point.y),
  }));
}

function elkLayoutOptions(
  mode: GraphLayoutMode,
  direction: GraphLayoutDirection,
  padding: number,
  nodeSpacing: number,
  layerSpacing: number,
  dag: boolean,
): Record<string, string> {
  const common = {
    "elk.algorithm": ELK_LAYOUT_ALGORITHM[mode],
    "elk.spacing.nodeNode": String(nodeSpacing),
    "elk.spacing.componentComponent": String(Math.max(nodeSpacing, 96)),
    "elk.padding": `[top=${padding},left=${padding},bottom=${padding},right=${padding}]`,
    "elk.separateConnectedComponents": "true",
  };

  if (mode === "dependency") {
    return {
      ...common,
      "elk.stress.desiredEdgeLength": String(Math.max(120, layerSpacing)),
      "elk.stress.iterationLimit": "400",
    };
  }

  if (mode === "state") {
    return {
      ...common,
      "elk.force.iterations": "600",
      "elk.force.repulsion": "8",
      "elk.aspectRatio": "1.45",
    };
  }

  if (mode === "radial") {
    return {
      ...common,
      "elk.direction": ELK_DIRECTION[direction],
      "elk.radial.radius": String(Math.max(140, layerSpacing)),
      "elk.radial.centerOnRoot": "true",
    };
  }

  return {
    ...common,
    "elk.direction": ELK_DIRECTION[direction],
    "elk.edgeRouting": "ORTHOGONAL",
    "elk.spacing.edgeNode": "28",
    "elk.spacing.edgeEdge": "14",
    "elk.spacing.portPort": "11",
    "elk.layered.spacing.nodeNodeBetweenLayers": String(layerSpacing),
    "elk.layered.spacing.edgeNodeBetweenLayers": "36",
    "elk.layered.spacing.edgeEdgeBetweenLayers": "18",
    "elk.layered.layering.strategy": "NETWORK_SIMPLEX",
    "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
    "elk.layered.nodePlacement.strategy": "BRANDES_KOEPF",
    "elk.layered.cycleBreaking.strategy": dag ? "DEPTH_FIRST" : "GREEDY",
    "elk.layered.portSortingStrategy": "INPUT_ORDER",
    "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
  };
}

function manualNodePosition(
  node: GraphNode,
  index: number,
  nodeCount: number,
  padding: number,
  nodeSpacing: number,
): { x: number; y: number } {
  const hasX = typeof node.x === "number" && Number.isFinite(node.x);
  const hasY = typeof node.y === "number" && Number.isFinite(node.y);
  if (hasX && hasY) {
    return { x: Math.round(node.x as number), y: Math.round(node.y as number) };
  }

  const columns = Math.max(1, Math.ceil(Math.sqrt(Math.max(1, nodeCount))));
  const width = node.width ?? estimateGraphNodeWidthForStyle(node);
  const height = node.height ?? estimateGraphNodeHeightForStyle(node);
  const column = index % columns;
  const row = Math.floor(index / columns);
  return {
    x: Math.round(padding + column * (width + nodeSpacing)),
    y: Math.round(padding + row * (height + nodeSpacing)),
  };
}

function layoutManualGraphDocument(
  graph: GraphData,
  layoutOptions: GraphLayoutOptions,
  direction: GraphLayoutDirection,
  padding: number,
  nodeSpacing: number,
): GraphData {
  const nextNodes = graph.nodes.map((node, index) => {
    const position = manualNodePosition(node, index, graph.nodes.length, padding, nodeSpacing);
    return {
      ...node,
      x: position.x,
      y: position.y,
      width: Math.round(node.width ?? estimateGraphNodeWidthForStyle(node)),
      height: Math.round(node.height ?? estimateGraphNodeHeightForStyle(node)),
    };
  });
  const nextConnections = graphConnections(graph).map((connection, index) => ({
    ...connection,
    id: connection.id || `edge-${index + 1}`,
    points: connection.points?.map((point) => ({
      x: Math.round(point.x),
      y: Math.round(point.y),
    })),
  }));
  const coloredConnections = colorGraphOverlappingRoutes({
    connections: nextConnections,
  });

  return {
    ...graph,
    layout: {
      ...graph.layout,
      ...layoutOptions,
      engine: "none",
      mode: "manual",
      direction,
    },
    nodes: nextNodes,
    connections: coloredConnections,
    links: coloredConnections,
  };
}

export async function layoutGraphDocument(
  source: GraphData,
  options: GraphLayoutOptions = {},
): Promise<GraphData> {
  const graph = normalizeGraphData(source);
  if (!graph.nodes.length) return graph;

  const layoutOptions = {
    ...graph.layout,
    ...options,
  };
  const mode = normalizeGraphLayoutMode(layoutOptions.mode);
  const direction = layoutOptions.direction ?? "right";
  const statePortPlacement = normalizeGraphStatePortPlacement(layoutOptions.statePortPlacement);
  const padding = layoutOptions.padding ?? 32;
  const nodeSpacing = layoutOptions.nodeSpacing ?? 84;
  const layerSpacing = layoutOptions.layerSpacing ?? 180;
  if (mode === "manual") {
    return layoutManualGraphDocument(graph, layoutOptions, direction, padding, nodeSpacing);
  }

  const elk = await getElk();
  const connections = graphConnections(graph);
  const dag = graphIsDag(graph);
  const hasStateNodeStyle = graph.nodes.some((node) =>
    normalizeGraphNodeStyle(node.nodeStyle ?? layoutOptions.nodeStyle) === "state",
  );
  const usePorts = mode === "flow" && !hasStateNodeStyle;
  const elkGraph: ElkNode = {
    id: "root",
    layoutOptions: elkLayoutOptions(mode, direction, padding, nodeSpacing, layerSpacing, dag),
    children: graph.nodes.map((node) => toElkNode(node, direction, usePorts)),
    edges: connections.map((connection, index) => toElkEdge(graph, connection, index, usePorts)),
  };

  const result = await elk.layout(elkGraph);
  const layoutNodeById = new Map((result.children ?? []).map((node) => [node.id, node]));
  const layoutEdgeById = new Map((result.edges ?? []).map((edge) => [edge.id, edge]));
  const nextNodes = graph.nodes.map((node) => {
    const layoutNode = layoutNodeById.get(node.id);
    return {
      ...node,
      x: Math.round(layoutNode?.x ?? node.x ?? 0),
      y: Math.round(layoutNode?.y ?? node.y ?? 0),
      width: Math.round(node.width ?? layoutNode?.width ?? estimateGraphNodeWidthForStyle(node)),
      height: Math.round(node.height ?? layoutNode?.height ?? estimateGraphNodeHeightForStyle(node)),
    };
  });
  const graphWithLayout = { ...graph, nodes: nextNodes };
  const nextConnections = connections.map((connection, index) => {
    const id = connection.id || `edge-${index + 1}`;
    const points = mode === "flow"
      ? edgePoints(layoutEdgeById.get(id) as ElkExtendedEdge | undefined)
      : undefined;
    const startNode = graphWithLayout.nodes.find((node) => node.id === connection.from.nodeId);
    const endNode = graphWithLayout.nodes.find((node) => node.id === connection.to.nodeId);
    if (points && points.length >= 2) {
      if (startNode) {
        points[0] = graphNodePortAnchor(startNode, "output", connection.from.portId, direction, statePortPlacement);
      }
      if (endNode) {
        points[points.length - 1] = graphNodePortAnchor(endNode, "input", connection.to.portId, direction, statePortPlacement);
      }
    }
    return {
      ...connection,
      id,
      points,
    };
  });
  const coloredConnections = colorGraphOverlappingRoutes({
    connections: nextConnections,
  });

  return {
    ...graph,
    layout: {
      ...graph.layout,
      ...layoutOptions,
      engine: "elk",
      mode,
      direction,
    },
    nodes: nextNodes,
    connections: coloredConnections,
    links: coloredConnections,
  };
}
