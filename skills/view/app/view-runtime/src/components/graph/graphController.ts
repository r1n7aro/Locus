import type {
  GraphConnectionValidation,
  GraphData,
  GraphLink,
  GraphNode,
} from "./graphTypes";

export interface GraphController {
  loadGraph?: () => GraphData | Promise<GraphData>;
  saveGraph?: (graph: GraphData) => unknown | Promise<unknown>;
  applyGraph?: (graph: GraphData) => unknown | Promise<unknown>;
  createNode?: (graph: GraphData) => GraphNode | null | undefined;
  validateConnection?: (
    connection: GraphLink,
    graph: GraphData,
  ) => GraphConnectionValidation;
  onGraphChange?: (graph: GraphData) => void;
}

export class GraphViewController implements GraphController {
  loadGraph(): GraphData {
    return { schema: "locus.graph.v1", nodes: [], connections: [] };
  }

  saveGraph(_graph: GraphData): void {
    // Default graph templates keep data in memory until a package overrides this method.
  }

  applyGraph(graph: GraphData): unknown | Promise<unknown> {
    return this.saveGraph(graph);
  }

  validateConnection(connection: GraphLink): GraphConnectionValidation {
    if (!connection.from.nodeId || !connection.to.nodeId) return false;
    if (
      connection.from.nodeId === connection.to.nodeId
      && (connection.from.portId ?? null) === (connection.to.portId ?? null)
    ) {
      return "Connection target matches source.";
    }
    return true;
  }
}

export function defineGraphView<T extends GraphController>(controller: T): T {
  return controller;
}
