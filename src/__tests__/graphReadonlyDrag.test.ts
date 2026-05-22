import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const source = readFileSync(resolve(process.cwd(), "src/components/graph/LocusGraphView.ts"), "utf8");

describe("LocusGraphView readonly dragging", () => {
  it("allows readonly node movement without reporting graph edits", () => {
    expect(source).toContain("function onNodePointerDown");
    expect(source).toContain("if (shouldIgnoreNodeDrag(event.target)) return;");
    expect(source).not.toContain("if (props.readonly || shouldIgnoreNodeDrag(event.target)) return;");
    expect(source).toContain("clearConnectionRoutesForNode(node.id);");
    expect(source).not.toContain("clearConnectionRoutes()");
    expect(source).not.toContain("rerouteConnectionsForNode(node.id)");
    expect(source).toContain("if (!props.readonly) dirty.value = true;");
    expect(source).toContain("if (didDrag && !props.readonly) notifyGraphChange();");
  });

  it("uses lightweight drag rendering for connected edges", () => {
    expect(source).toContain("function connectionBezierPath");
    expect(source).toContain("draggingNodeId.value && connectionTouchesNode(connection, draggingNodeId.value)");
    expect(source).toContain("return connectionBezierPath(start, end);");
    expect(source).toContain("window.requestAnimationFrame(flush)");
    expect(source).toContain("return graphRouteColorIndexById({ connections: graphConnections(graph) });");
    expect(source).not.toContain("edgeVersion.value;\n        return graphRouteColorIndexById");
  });

  it("renders connected port fills and configurable node-level ports", () => {
    expect(source).toContain("const portColorIndexByKey = computed");
    expect(source).toContain("connected ? \"connected\" : \"\"");
    expect(source).toContain("connected ? `route-color-${colorIndex}` : \"\"");
    expect(source).toContain("function nodePortsConfig");
    expect(source).toContain("graph.layout?.nodePorts ?? props.layoutOptions.nodePorts ?? true");
    expect(source).toContain("hasNodeInputPort ? renderPort(node, \"input\", null) : null");
    expect(source).toContain("hasNodeOutputPort ? renderPort(node, \"output\", null) : null");
  });
});
