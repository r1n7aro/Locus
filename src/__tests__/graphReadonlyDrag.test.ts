import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const graphSource = readFileSync(resolve(process.cwd(), "src/components/graph/LocusGraphView.ts"), "utf8");
const graphStylesSource = readFileSync(resolve(process.cwd(), "src/components/graph/graphStyles.ts"), "utf8");
const canvasSource = readFileSync(resolve(process.cwd(), "src/components/canvas/LocusCanvasView.ts"), "utf8");

describe("LocusGraphView readonly dragging", () => {
  it("delegates node movement to CanvasView without reporting readonly graph edits", () => {
    expect(graphSource).toContain("import { CanvasView");
    expect(graphSource).toContain("function onCanvasItemDragStart");
    expect(graphSource).toContain("clearConnectionRoutesForNode(event.item.id);");
    expect(graphSource).not.toContain("clearConnectionRoutes()");
    expect(graphSource).not.toContain("rerouteConnectionsForNode(node.id)");
    expect(graphSource).toContain("function onCanvasItemMove()");
    expect(graphSource).toContain("if (!props.readonly) dirty.value = true;");
    expect(graphSource).toContain("if (event.didDrag && !props.readonly) notifyGraphChange();");
    expect(graphSource).toContain("moveReadonly: true");
    expect(canvasSource).toContain("function onItemPointerDown");
    expect(canvasSource).toContain("function effectiveBehavior");
    expect(canvasSource).toContain("props.moveReadonly");
    expect(canvasSource).toContain("if (!behavior.allowMove || !nextSelection.includes(item.id)) return;");
    expect(canvasSource).toContain("dragItem.x = Math.round");
    expect(canvasSource).toContain("dragItem.y = Math.round");
  });

  it("uses lightweight drag rendering for connected edges", () => {
    expect(graphSource).toContain("function connectionBezierPath");
    expect(graphSource).toContain("draggingNodeId.value && connectionTouchesNode(connection, draggingNodeId.value)");
    expect(graphSource).toContain("return connectionBezierPath(start, end, connectionUsesVerticalPorts(connection));");
    expect(graphSource).toContain("window.requestAnimationFrame(flush)");
    expect(graphSource).toContain("return graphRouteColorIndexById({ connections: graphConnections(graph) });");
    expect(graphSource).not.toContain("edgeVersion.value;\n        return graphRouteColorIndexById");
  });

  it("renders directed edge direction as a light middle chevron", () => {
    expect(graphSource).toContain("function graphDirectionChevronPath");
    expect(graphSource).toContain("function connectionDirectionPath");
    expect(graphSource).toContain("\"locus-graph-edge-direction\"");
    expect(graphSource).not.toContain("\"marker-end\"");
    expect(graphStylesSource).toContain(".locus-graph-edge-direction");
    expect(graphStylesSource).toContain("opacity: 0.58");
  });

  it("renders readonly parameter formulas as wrapping node text", () => {
    expect(graphSource).toContain("function graphFormulaTokens");
    expect(graphSource).toContain("function renderGraphFormulaCode");
    expect(graphSource).toContain("disabled && type !== \"boolean\" && type !== \"color\"");
    expect(graphSource).toContain("\"locus-graph-parameter-value\"");
    expect(graphSource).toContain("\"locus-graph-formula-token\"");
    expect(graphStylesSource).toContain(".locus-graph-parameter-value");
    expect(graphStylesSource).toContain("font-family: var(--font-mono-inline)");
    expect(graphStylesSource).toContain("overflow-wrap: anywhere");
    expect(graphStylesSource).toContain(".locus-graph-formula-token.token-operator");
    expect(graphStylesSource).toContain(".locus-graph-parameters.align-output .locus-graph-parameter {\n  width: 100%;");
  });

  it("renders connected port fills and configurable node-level ports", () => {
    expect(graphSource).toContain("const portColorIndexByKey = computed");
    expect(graphSource).toContain("connected ? \"connected\" : \"\"");
    expect(graphSource).toContain("connected ? `route-color-${colorIndex}` : \"\"");
    expect(graphSource).toContain("function nodePortsConfig");
    expect(graphSource).toContain("graph.layout?.nodePorts ?? props.layoutOptions.nodePorts ?? true");
    expect(graphSource).toContain("hasNodeInputPort ? renderPort(node, \"input\", null, inputSide) : null");
    expect(graphSource).toContain("hasNodeOutputPort ? renderPort(node, \"output\", null, outputSide) : null");
  });

  it("uses shared UI controls in the graph toolbar", () => {
    expect(graphSource).toContain('import BaseButton from "../ui/BaseButton.vue";');
    expect(graphSource).toContain('import BaseDropdown from "../ui/BaseDropdown.vue";');
    expect(graphSource).toContain("return h(BaseButton");
    expect(graphSource).toContain("h(BaseDropdown");
    expect(graphSource).not.toContain('h("select", {\n              class: "locus-graph-layout-mode"');
    expect(graphStylesSource).not.toContain(".locus-graph-actions button");
  });

  it("writes graph lifecycle failures to the View frontend log", () => {
    expect(graphSource).toContain('console.error("[GraphView] loadGraph failed", loadError);');
    expect(graphSource).toContain('console.error("[GraphView] saveGraph failed", saveError);');
    expect(graphSource).toContain('console.error("[GraphView] applyGraph failed", applyError);');
    expect(graphSource).toContain('console.error("[GraphView] layout failed", layoutError);');
  });

  it("lets explicit layout actions run even when automatic layout is off", () => {
    expect(graphSource).toContain("function shouldAutoLayout");
    expect(graphSource).toContain("if (force) return true;");
    expect(graphSource).toContain("await layoutCurrentGraph(true, true);");
  });
});
