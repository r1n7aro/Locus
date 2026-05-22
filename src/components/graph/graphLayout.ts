import type { ElkExtendedEdge, ElkNode, ElkPort } from "elkjs";
import type {
  GraphData,
  GraphLayoutDirection,
  GraphLayoutOptions,
  GraphLink,
  GraphNode,
  GraphPort,
} from "./graphTypes";
import {
  GRAPH_NODE_BODY_PADDING_TOP,
  GRAPH_NODE_HEADER_HEIGHT,
  GRAPH_NODE_PORT_ID,
  GRAPH_NODE_WIDTH,
  GRAPH_PORT_ROW_PITCH,
  GRAPH_PORT_SIZE,
  colorGraphOverlappingRoutes,
  estimateGraphNodeHeight,
  estimateGraphNodeWidth,
  graphConnections,
  graphIsDag,
  graphNodePortAnchor,
  normalizeGraphData,
} from "./graphTypes";

type ElkConstructor = typeof import("elkjs").default;

const ELK_DIRECTION: Record<GraphLayoutDirection, string> = {
  right: "RIGHT",
  left: "LEFT",
  down: "DOWN",
  up: "UP",
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

function toElkNode(node: GraphNode, layoutDirection: GraphLayoutDirection): ElkNode {
  const width = node.width ?? estimateGraphNodeWidth(node);
  const height = node.height ?? estimateGraphNodeHeight(node);
  return {
    id: node.id,
    width,
    height,
    ports: elkPorts(node, layoutDirection, width),
    layoutOptions: {
      "elk.portConstraints": "FIXED_POS",
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

function toElkEdge(graph: GraphData, connection: GraphLink, index: number): ElkExtendedEdge {
  return {
    id: connection.id || `edge-${index + 1}`,
    sources: [endpointShapeId(graph, connection.from, "output")],
    targets: [endpointShapeId(graph, connection.to, "input")],
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
  const direction = layoutOptions.direction ?? "right";
  const padding = layoutOptions.padding ?? 32;
  const nodeSpacing = layoutOptions.nodeSpacing ?? 84;
  const layerSpacing = layoutOptions.layerSpacing ?? 180;
  const elk = await getElk();
  const connections = graphConnections(graph);
  const dag = graphIsDag(graph);
  const elkGraph: ElkNode = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": ELK_DIRECTION[direction],
      "elk.edgeRouting": "ORTHOGONAL",
      "elk.spacing.nodeNode": String(nodeSpacing),
      "elk.spacing.edgeNode": "28",
      "elk.spacing.edgeEdge": "14",
      "elk.spacing.portPort": "11",
      "elk.spacing.componentComponent": String(Math.max(nodeSpacing, 96)),
      "elk.layered.spacing.nodeNodeBetweenLayers": String(layerSpacing),
      "elk.layered.spacing.edgeNodeBetweenLayers": "36",
      "elk.layered.spacing.edgeEdgeBetweenLayers": "18",
      "elk.layered.layering.strategy": "NETWORK_SIMPLEX",
      "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
      "elk.layered.nodePlacement.strategy": "BRANDES_KOEPF",
      "elk.layered.cycleBreaking.strategy": dag ? "DEPTH_FIRST" : "GREEDY",
      "elk.layered.portSortingStrategy": "INPUT_ORDER",
      "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
      "elk.padding": `[top=${padding},left=${padding},bottom=${padding},right=${padding}]`,
      "elk.separateConnectedComponents": "true",
    },
    children: graph.nodes.map((node) => toElkNode(node, direction)),
    edges: connections.map((connection, index) => toElkEdge(graph, connection, index)),
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
      width: Math.round(node.width ?? layoutNode?.width ?? GRAPH_NODE_WIDTH),
      height: Math.round(node.height ?? layoutNode?.height ?? estimateGraphNodeHeight(node)),
    };
  });
  const graphWithLayout = { ...graph, nodes: nextNodes };
  const nextConnections = connections.map((connection, index) => {
    const id = connection.id || `edge-${index + 1}`;
    const points = edgePoints(layoutEdgeById.get(id) as ElkExtendedEdge | undefined);
    const startNode = graphWithLayout.nodes.find((node) => node.id === connection.from.nodeId);
    const endNode = graphWithLayout.nodes.find((node) => node.id === connection.to.nodeId);
    if (points && points.length >= 2) {
      if (startNode) points[0] = graphNodePortAnchor(startNode, "output", connection.from.portId);
      if (endNode) points[points.length - 1] = graphNodePortAnchor(endNode, "input", connection.to.portId);
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
      direction,
    },
    nodes: nextNodes,
    connections: coloredConnections,
    links: coloredConnections,
  };
}
