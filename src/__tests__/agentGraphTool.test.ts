import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  agentGraphToolRequestIdFromLocation,
  buildAgentGraphToolWindowUrl,
  isAgentGraphToolWindowLocation,
} from "../services/agentGraphTool";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("agentGraphTool service", () => {
  it("builds and parses graph window URLs", () => {
    const url = buildAgentGraphToolWindowUrl("graph-1");

    expect(url).toContain("/agent-graph?");
    expect(url).toContain("agentGraph=1");
    expect(agentGraphToolRequestIdFromLocation(url.slice(url.indexOf("?")))).toBe("graph-1");
  });

  it("detects dedicated graph tool windows", () => {
    expect(isAgentGraphToolWindowLocation({ pathname: "/agent-graph", search: "?id=1" })).toBe(true);
    expect(isAgentGraphToolWindowLocation({ pathname: "/", search: "?agentGraph=1&id=1" })).toBe(true);
    expect(isAgentGraphToolWindowLocation({ pathname: "/", search: "" })).toBe(false);
  });

  it("keeps graph_view description out of the user-facing graph flow", () => {
    const definition = JSON.parse(read("tools/graph_view.json"));

    expect(definition.parameters.properties.description).toBeUndefined();
    expect(read("src/components/AgentGraphToolWindow.vue")).not.toContain("agent-graph-description");
    expect(read("src/components/ToolCallBlock.vue")).toContain("GRAPH_VIEW_HIDDEN_ARG_KEYS");
  });

  it("keeps the graph window title out of the embedded graph toolbar", () => {
    const source = read("src/components/AgentGraphToolWindow.vue");

    expect(source).toContain('title=""');
    expect(source).not.toContain(':title="payload.title"');
  });

  it("documents manual layout image return for vision-capable endpoints", () => {
    const definition = JSON.parse(read("tools/graph_view.json"));

    expect(definition.parameters.properties.returnImage.type).toBe("boolean");
    expect(definition.parameters.properties.returnImage.description).toContain("image understanding");
    expect(definition.parameters.properties.layout.properties.mode.enum).toContain("manual");
  });

  it("documents Unreal and Unity style state nodes", () => {
    const definition = JSON.parse(read("tools/graph_view.json"));

    expect(definition.parameters.properties.layout.properties.nodeStyle.enum).toEqual(["blueprint", "state"]);
    expect(definition.parameters.properties.nodes.items.properties.nodeStyle.enum).toEqual(["blueprint", "state"]);
    expect(definition.description).toContain("layout.nodeStyle=state");
  });

  it("documents directed links and vertical state ports", () => {
    const definition = JSON.parse(read("tools/graph_view.json"));

    expect(definition.parameters.properties.layout.properties.directed.type).toBe("boolean");
    expect(definition.parameters.properties.layout.properties.statePortPlacement.enum).toEqual([
      "auto",
      "horizontal",
      "vertical",
    ]);
    expect(definition.parameters.properties.links.items.properties.directed.type).toBe("boolean");
    expect(definition.description).toContain("layout.directed=true");
  });
});
