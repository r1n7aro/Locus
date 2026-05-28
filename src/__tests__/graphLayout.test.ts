import { describe, expect, it } from "vitest";
import {
  GRAPH_NODE_MAX_WIDTH,
  GRAPH_NODE_MIN_WIDTH,
  colorGraphOverlappingRoutes,
  estimateGraphNodeWidth,
  graphNodePortAnchor,
  graphConnections,
  graphIsDag,
  graphRoutePointsWithAnchors,
  graphStateNodePortSide,
  locusGraphCss,
  layoutGraphDocument,
  normalizeGraphLayoutMode,
  normalizeGraphNodeStyle,
  normalizeGraphStatePortPlacement,
  normalizeGraphData,
} from "../components/graph";

describe("graphLayout", () => {
  it("auto lays out nodes and routes links with ports", async () => {
    const graph = await layoutGraphDocument({
      schema: "locus.graph.v1",
      layout: { direction: "right" },
      nodes: [
        {
          id: "shader",
          title: "Shader",
          outputs: [{ id: "color", label: "Color" }],
        },
        {
          id: "multiply",
          title: "Multiply",
          inputs: [{ id: "a", label: "A" }],
          outputs: [{ id: "result", label: "Result" }],
        },
        {
          id: "output",
          title: "Output",
          inputs: [{ id: "base", label: "Base" }],
        },
      ],
      links: [
        {
          id: "shader-multiply",
          from: { nodeId: "shader", portId: "color" },
          to: { nodeId: "multiply", portId: "a" },
        },
        {
          id: "multiply-output",
          from: { nodeId: "multiply", portId: "result" },
          to: { nodeId: "output", portId: "base" },
        },
      ],
    });

    const shader = graph.nodes.find((node) => node.id === "shader");
    const multiply = graph.nodes.find((node) => node.id === "multiply");
    const output = graph.nodes.find((node) => node.id === "output");
    const connections = graphConnections(graph);

    expect(shader?.x).toEqual(expect.any(Number));
    expect(multiply?.x).toEqual(expect.any(Number));
    expect(output?.x).toEqual(expect.any(Number));
    expect((multiply?.x ?? 0)).toBeGreaterThan(shader?.x ?? 0);
    expect((output?.x ?? 0)).toBeGreaterThan(multiply?.x ?? 0);
    expect(shader?.width).toBeGreaterThanOrEqual(GRAPH_NODE_MIN_WIDTH);
    expect(multiply?.width).toBeGreaterThanOrEqual(GRAPH_NODE_MIN_WIDTH);
    expect(output?.width).toBeGreaterThanOrEqual(GRAPH_NODE_MIN_WIDTH);
    expect(connections).toHaveLength(2);
    expect(connections[0].points?.length).toBeGreaterThanOrEqual(2);
    expect(graph.links).toBe(graph.connections);
  });

  it("routes different ports on the same node to different anchor positions", async () => {
    const graph = await layoutGraphDocument({
      schema: "locus.graph.v1",
      layout: { direction: "right" },
      nodes: [
        {
          id: "source",
          title: "Source",
          outputs: [
            { id: "top", label: "Top" },
            { id: "bottom", label: "Bottom" },
          ],
        },
        {
          id: "topTarget",
          title: "Top Target",
          inputs: [{ id: "in", label: "In" }],
        },
        {
          id: "bottomTarget",
          title: "Bottom Target",
          inputs: [{ id: "in", label: "In" }],
        },
      ],
      links: [
        {
          id: "source-top",
          from: { nodeId: "source", portId: "top" },
          to: { nodeId: "topTarget", portId: "in" },
        },
        {
          id: "source-bottom",
          from: { nodeId: "source", portId: "bottom" },
          to: { nodeId: "bottomTarget", portId: "in" },
        },
      ],
    });

    const connections = graphConnections(graph);
    const source = graph.nodes.find((node) => node.id === "source");
    const topStart = connections.find((connection) => connection.id === "source-top")?.points?.[0];
    const bottomStart = connections.find((connection) => connection.id === "source-bottom")?.points?.[0];
    const topAnchor = source ? graphNodePortAnchor(source, "output", "top") : null;

    expect(topStart?.x).toEqual(expect.any(Number));
    expect(bottomStart?.x).toEqual(expect.any(Number));
    expect(Math.abs((topStart?.y ?? 0) - (bottomStart?.y ?? 0))).toBeGreaterThan(8);
    expect(Math.abs((topStart?.x ?? 0) - (topAnchor?.x ?? 0))).toBeLessThanOrEqual(1);
    expect(Math.abs((topStart?.y ?? 0) - (topAnchor?.y ?? 0))).toBeLessThanOrEqual(1);
  });

  it("keeps routed paths orthogonal when endpoints move", () => {
    const points = graphRoutePointsWithAnchors(
      [
        { x: 240, y: 70 },
        { x: 300, y: 70 },
        { x: 300, y: 160 },
        { x: 420, y: 160 },
      ],
      { x: 260, y: 120 },
      { x: 460, y: 210 },
      "right",
    );

    expect(points).toEqual([
      { x: 260, y: 120 },
      { x: 300, y: 120 },
      { x: 300, y: 210 },
      { x: 460, y: 210 },
    ]);
  });

  it("creates an orthogonal route when a connection has no saved route points", () => {
    const points = graphRoutePointsWithAnchors(
      undefined,
      { x: 120, y: 40 },
      { x: 360, y: 140 },
      "right",
    );

    expect(points).toEqual([
      { x: 120, y: 40 },
      { x: 240, y: 40 },
      { x: 240, y: 140 },
      { x: 360, y: 140 },
    ]);
  });

  it("assigns different color indexes to overlapping route segments", () => {
    const connections = colorGraphOverlappingRoutes({
      connections: [
        {
          id: "top",
          from: { nodeId: "a", portId: "out" },
          to: { nodeId: "b", portId: "in" },
          points: [
            { x: 0, y: 20 },
            { x: 120, y: 20 },
            { x: 120, y: 80 },
          ],
        },
        {
          id: "bottom",
          from: { nodeId: "a", portId: "out2" },
          to: { nodeId: "c", portId: "in" },
          points: [
            { x: 30, y: 20 },
            { x: 150, y: 20 },
            { x: 150, y: 120 },
          ],
        },
        {
          id: "clear",
          from: { nodeId: "d", portId: "out" },
          to: { nodeId: "e", portId: "in" },
          points: [
            { x: 0, y: 200 },
            { x: 120, y: 200 },
          ],
        },
      ],
    });

    const top = connections.find((connection) => connection.id === "top");
    const bottom = connections.find((connection) => connection.id === "bottom");
    const clear = connections.find((connection) => connection.id === "clear");

    expect(top?.style?.hasOverlap).toBe(true);
    expect(bottom?.style?.hasOverlap).toBe(true);
    expect(top?.style?.colorIndex).not.toBe(bottom?.style?.colorIndex);
    expect(clear?.style?.colorIndex).toBeUndefined();
  });

  it("keeps fan-out links from the same output port on the same color", () => {
    const connections = colorGraphOverlappingRoutes({
      connections: [
        {
          id: "fan-a",
          from: { nodeId: "source", portId: "out" },
          to: { nodeId: "targetA", portId: "in" },
          points: [
            { x: 0, y: 20 },
            { x: 150, y: 20 },
            { x: 150, y: 80 },
          ],
        },
        {
          id: "fan-b",
          from: { nodeId: "source", portId: "out" },
          to: { nodeId: "targetB", portId: "in" },
          points: [
            { x: 20, y: 20 },
            { x: 170, y: 20 },
            { x: 170, y: 120 },
          ],
        },
        {
          id: "other",
          from: { nodeId: "source", portId: "other" },
          to: { nodeId: "targetC", portId: "in" },
          points: [
            { x: 40, y: 20 },
            { x: 190, y: 20 },
            { x: 190, y: 160 },
          ],
        },
      ],
    });

    const fanA = connections.find((connection) => connection.id === "fan-a");
    const fanB = connections.find((connection) => connection.id === "fan-b");
    const other = connections.find((connection) => connection.id === "other");

    expect(fanA?.style?.hasOverlap).toBe(true);
    expect(fanA?.style?.colorIndex).toBe(fanB?.style?.colorIndex);
    expect(other?.style?.colorIndex).not.toBe(fanA?.style?.colorIndex);
  });

  it("leaves unrelated routes on default color even when they come from the same node", () => {
    const connections = colorGraphOverlappingRoutes({
      connections: [
        {
          id: "world",
          from: { nodeId: "source", portId: "world" },
          to: { nodeId: "depth", portId: "screen" },
          points: [
            { x: 0, y: 20 },
            { x: 140, y: 20 },
          ],
        },
        {
          id: "screen",
          from: { nodeId: "source", portId: "screen" },
          to: { nodeId: "depth", portId: "uv" },
          points: [
            { x: 0, y: 40 },
            { x: 140, y: 40 },
          ],
        },
        {
          id: "mesh",
          from: { nodeId: "source", portId: "mesh" },
          to: { nodeId: "distort", portId: "uv" },
          points: [
            { x: 0, y: 60 },
            { x: 140, y: 60 },
          ],
        },
      ],
    });

    const colors = connections.map((connection) => connection.style?.colorIndex);

    expect(colors).toEqual([undefined, undefined, undefined]);
  });

  it("assigns different colors to nearby overlapping routes globally", () => {
    const connections = colorGraphOverlappingRoutes({
      connections: [
        {
          id: "main-light",
          from: { nodeId: "lighting", portId: "main" },
          to: { nodeId: "outline", portId: "light" },
          points: [
            { x: 120, y: 0 },
            { x: 120, y: 160 },
          ],
        },
        {
          id: "interaction-light",
          from: { nodeId: "interaction", portId: "light" },
          to: { nodeId: "water", portId: "light" },
          points: [
            { x: 123, y: 40 },
            { x: 123, y: 220 },
          ],
        },
        {
          id: "screen-uv",
          from: { nodeId: "base", portId: "uv" },
          to: { nodeId: "outline", portId: "uv" },
          points: [
            { x: 0, y: 300 },
            { x: 160, y: 300 },
          ],
        },
      ],
    });

    const mainLight = connections.find((connection) => connection.id === "main-light");
    const interactionLight = connections.find((connection) => connection.id === "interaction-light");
    const screenUv = connections.find((connection) => connection.id === "screen-uv");

    expect(mainLight?.style?.hasOverlap).toBe(true);
    expect(interactionLight?.style?.hasOverlap).toBe(true);
    expect(mainLight?.style?.colorIndex).not.toBe(interactionLight?.style?.colorIndex);
    expect(screenUv?.style?.colorIndex).toBeUndefined();
  });

  it("assigns different colors to close routes from different ports on the same source", () => {
    const connections = colorGraphOverlappingRoutes({
      connections: [
        {
          id: "screen",
          from: { nodeId: "base-input", portId: "screen" },
          to: { nodeId: "water-depth", portId: "screen" },
          points: [
            { x: 0, y: 40 },
            { x: 120, y: 40 },
            { x: 120, y: 180 },
          ],
        },
        {
          id: "mesh",
          from: { nodeId: "base-input", portId: "mesh" },
          to: { nodeId: "distort", portId: "mesh" },
          points: [
            { x: 0, y: 64 },
            { x: 134, y: 64 },
            { x: 134, y: 180 },
          ],
        },
      ],
    });

    const screen = connections.find((connection) => connection.id === "screen");
    const mesh = connections.find((connection) => connection.id === "mesh");

    expect(screen?.style?.hasOverlap).toBe(true);
    expect(mesh?.style?.hasOverlap).toBe(true);
    expect(screen?.style?.colorIndex).not.toBe(mesh?.style?.colorIndex);
  });

  it("assigns different colors when route segments cross each other", () => {
    const connections = colorGraphOverlappingRoutes({
      connections: [
        {
          id: "world-position",
          from: { nodeId: "base-input", portId: "world" },
          to: { nodeId: "water-depth", portId: "world" },
          points: [
            { x: 0, y: 80 },
            { x: 180, y: 80 },
          ],
        },
        {
          id: "mesh-uv0",
          from: { nodeId: "base-input", portId: "mesh" },
          to: { nodeId: "distort", portId: "mesh" },
          points: [
            { x: 120, y: 140 },
            { x: 120, y: 40 },
            { x: 220, y: 40 },
          ],
        },
      ],
    });

    const worldPosition = connections.find((connection) => connection.id === "world-position");
    const meshUv0 = connections.find((connection) => connection.id === "mesh-uv0");

    expect(worldPosition?.style?.hasOverlap).toBe(true);
    expect(meshUv0?.style?.hasOverlap).toBe(true);
    expect(worldPosition?.style?.colorIndex).not.toBe(meshUv0?.style?.colorIndex);
  });

  it("uses distinct theme families for the first graph route colors", () => {
    const css = locusGraphCss();

    expect(css).toContain("--locus-graph-edge-color-0: color-mix(in srgb, var(--accent-color)");
    expect(css).toContain("--locus-graph-edge-color-1: color-mix(in srgb, var(--status-warn-fg)");
    expect(css).toContain("--locus-graph-edge-color-2: color-mix(in srgb, var(--status-good-fg)");
    expect(css).toContain("--locus-graph-edge-color-3: color-mix(in srgb, var(--status-danger-fg)");
  });

  it("uses route colors to fill connected ports", () => {
    const css = locusGraphCss();

    expect(css).toContain(".locus-graph-port.route-color-0");
    expect(css).toContain(".locus-graph-port.route-color-5");
    expect(css).toContain(".locus-graph-port.connected");
    expect(css).toContain("background: var(--locus-graph-port-fill)");
  });

  it("keeps generated node sizes large enough for labels and parameters", () => {
    const graph = normalizeGraphData({
      nodes: [
        {
          id: "final",
          width: 24,
          title: "Forward 输出",
          subtitle: "PixelForward Pass",
          inputs: [
            { id: "color", label: "Color" },
            { id: "light", label: "MainLight" },
            { id: "alpha", label: "Alpha" },
          ],
          parameters: [
            { id: "lightmode", label: "LightMode", type: "string", value: "PixelForward", readOnly: true },
            { id: "pass", label: "额外 Pass", type: "string", value: "Forward", readOnly: true },
          ],
        },
      ],
      links: [],
    });

    expect(graph.nodes[0].width).toBeGreaterThanOrEqual(GRAPH_NODE_MIN_WIDTH);
    expect(graph.nodes[0].width).toBe(estimateGraphNodeWidth(graph.nodes[0]));
    expect(graph.nodes[0].height).toBeGreaterThan(112);
  });

  it("expands graph nodes for long parameter formulas", () => {
    const graph = normalizeGraphData({
      nodes: [
        {
          id: "gerstner",
          title: "Gerstner Waves",
          parameters: [
            {
              id: "nodes",
              label: "Nodes",
              type: "string",
              value: "Position + Time + WaveAmplitude/WavePeriod + Direction + Steepness + PhaseOffset + NormalizedDistance",
              readOnly: true,
            },
          ],
        },
      ],
      links: [],
    });

    expect(graph.nodes[0].width).toBeGreaterThan(420);
    expect(graph.nodes[0].width).toBeLessThanOrEqual(GRAPH_NODE_MAX_WIDTH);
    expect(graph.nodes[0].height).toBeGreaterThan(112);
  });

  it("shrinks stale generated graph node heights back toward content", () => {
    const graph = normalizeGraphData({
      nodes: [
        {
          id: "compact",
          height: 260,
          parameters: [
            { id: "formula", label: "Formula", type: "string", value: "A + B", readOnly: true },
          ],
        },
      ],
      links: [],
    });

    expect(graph.nodes[0].height).toBeLessThan(160);
  });

  it("keeps graph-level node port display options", () => {
    const graph = normalizeGraphData({
      layout: {
        nodePorts: { input: false, output: true },
      },
      nodes: [{ id: "source" }],
      links: [],
    });

    expect(graph.layout?.nodePorts).toEqual({ input: false, output: true });
  });

  it("detects whether a graph is a DAG before layout", () => {
    expect(graphIsDag({
      nodes: [{ id: "a" }, { id: "b" }, { id: "c" }],
      links: [
        { from: { nodeId: "a" }, to: { nodeId: "b" } },
        { from: { nodeId: "b" }, to: { nodeId: "c" } },
      ],
    })).toBe(true);

    expect(graphIsDag({
      nodes: [{ id: "a" }, { id: "b" }, { id: "c" }],
      links: [
        { from: { nodeId: "a" }, to: { nodeId: "b" } },
        { from: { nodeId: "b" }, to: { nodeId: "c" } },
        { from: { nodeId: "c" }, to: { nodeId: "a" } },
      ],
    })).toBe(false);
  });

  it("normalizes layout mode aliases", () => {
    expect(normalizeGraphLayoutMode("stress")).toBe("dependency");
    expect(normalizeGraphLayoutMode("state-machine")).toBe("state");
    expect(normalizeGraphLayoutMode("circle")).toBe("radial");
    expect(normalizeGraphLayoutMode("fixed")).toBe("manual");
    expect(normalizeGraphLayoutMode("unknown")).toBe("flow");
  });

  it("normalizes node style aliases", () => {
    expect(normalizeGraphNodeStyle("state-machine")).toBe("state");
    expect(normalizeGraphNodeStyle("unreal")).toBe("state");
    expect(normalizeGraphNodeStyle("unknown")).toBe("blueprint");
  });

  it("normalizes state port placement aliases", () => {
    expect(normalizeGraphStatePortPlacement("top-bottom")).toBe("vertical");
    expect(normalizeGraphStatePortPlacement("sides")).toBe("horizontal");
    expect(normalizeGraphStatePortPlacement("unknown")).toBe("auto");
  });

  it("keeps directed graph link configuration", () => {
    const graph = normalizeGraphData({
      layout: { directed: true },
      nodes: [{ id: "source" }, { id: "target" }],
      links: [
        { id: "default", from: { nodeId: "source" }, to: { nodeId: "target" } },
        { id: "undirected", from: { nodeId: "target" }, to: { nodeId: "source" }, directed: false },
      ],
    });

    expect(graph.layout?.directed).toBe(true);
    expect(graphConnections(graph).find((connection) => connection.id === "undirected")?.directed).toBe(false);
  });

  it("supports dependency layout mode without persisted orthogonal routes", async () => {
    const graph = await layoutGraphDocument({
      schema: "locus.graph.v1",
      layout: { mode: "dependency", direction: "right" },
      nodes: [
        { id: "scene", title: "Scene" },
        { id: "prefab", title: "Prefab" },
        { id: "material", title: "Material" },
        { id: "texture", title: "Texture" },
      ],
      links: [
        { id: "scene-prefab", from: { nodeId: "scene" }, to: { nodeId: "prefab" } },
        { id: "prefab-material", from: { nodeId: "prefab" }, to: { nodeId: "material" } },
        { id: "material-texture", from: { nodeId: "material" }, to: { nodeId: "texture" } },
        { id: "scene-material", from: { nodeId: "scene" }, to: { nodeId: "material" } },
      ],
    });

    expect(graph.layout?.mode).toBe("dependency");
    expect(graph.nodes.every((node) => Number.isFinite(node.x) && Number.isFinite(node.y))).toBe(true);
    expect(graphConnections(graph).every((connection) => connection.points === undefined)).toBe(true);
  });

  it("supports state layout mode for cyclic graphs", async () => {
    const graph = await layoutGraphDocument({
      schema: "locus.graph.v1",
      layout: { mode: "state" },
      nodes: [
        { id: "idle", title: "Idle" },
        { id: "chase", title: "Chase" },
        { id: "attack", title: "Attack" },
        { id: "recover", title: "Recover" },
      ],
      links: [
        { id: "idle-chase", from: { nodeId: "idle" }, to: { nodeId: "chase" } },
        { id: "chase-attack", from: { nodeId: "chase" }, to: { nodeId: "attack" } },
        { id: "attack-recover", from: { nodeId: "attack" }, to: { nodeId: "recover" } },
        { id: "recover-idle", from: { nodeId: "recover" }, to: { nodeId: "idle" } },
      ],
    });

    expect(graph.layout?.mode).toBe("state");
    expect(graph.nodes.every((node) => Number.isFinite(node.x) && Number.isFinite(node.y))).toBe(true);
    expect(graphConnections(graph).every((connection) => connection.points === undefined)).toBe(true);
  });

  it("supports manual layout mode with preserved coordinates", async () => {
    const graph = await layoutGraphDocument({
      schema: "locus.graph.v1",
      layout: { mode: "manual", auto: "off" },
      nodes: [
        { id: "root", title: "Root", x: 320, y: 140 },
        { id: "missing", title: "Missing position" },
      ],
      links: [
        { id: "root-missing", from: { nodeId: "root" }, to: { nodeId: "missing" } },
      ],
    });

    const root = graph.nodes.find((node) => node.id === "root");
    const missing = graph.nodes.find((node) => node.id === "missing");

    expect(graph.layout?.mode).toBe("manual");
    expect(graph.layout?.engine).toBe("none");
    expect(root?.x).toBe(320);
    expect(root?.y).toBe(140);
    expect(missing?.x).toEqual(expect.any(Number));
    expect(missing?.y).toEqual(expect.any(Number));
    expect(graphConnections(graph)[0].points).toBeUndefined();
  });

  it("supports compact state machine node style", () => {
    const graph = normalizeGraphData({
      layout: { nodeStyle: "state" },
      nodes: [
        {
          id: "idle",
          title: "Idle",
          subtitle: "Default State",
        },
      ],
      links: [],
    });

    const node = graph.nodes[0];
    const inputAnchor = graphNodePortAnchor(node, "input");
    const outputAnchor = graphNodePortAnchor(node, "output");

    expect(node.nodeStyle).toBe("state");
    expect(node.height).toBeLessThan(90);
    expect(node.width).toBeLessThan(260);
    expect(inputAnchor.y).toBe((node.y ?? 0) + Math.round((node.height ?? 0) / 2));
    expect(outputAnchor.x).toBe((node.x ?? 0) + (node.width ?? 0));
  });

  it("supports top and bottom ports for state nodes", () => {
    const graph = normalizeGraphData({
      layout: { nodeStyle: "state", direction: "down", statePortPlacement: "vertical" },
      nodes: [
        {
          id: "idle",
          title: "Idle",
          x: 40,
          y: 80,
        },
      ],
      links: [],
    });

    const node = graph.nodes[0];
    const inputAnchor = graphNodePortAnchor(node, "input", null, "down", "vertical");
    const outputAnchor = graphNodePortAnchor(node, "output", null, "down", "vertical");

    expect(graphStateNodePortSide("input", "down", "vertical")).toBe("top");
    expect(graphStateNodePortSide("output", "down", "vertical")).toBe("bottom");
    expect(inputAnchor.x).toBe((node.x ?? 0) + Math.round((node.width ?? 0) / 2));
    expect(inputAnchor.y).toBe(node.y);
    expect(outputAnchor.x).toBe(inputAnchor.x);
    expect(outputAnchor.y).toBe((node.y ?? 0) + (node.height ?? 0));
  });
});
