pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { GraphView, GraphViewController, defineGraphView, view } from "@locus/view-runtime";

function fallbackGraph() {
  return {
    layout: { auto: "missing", direction: "right" },
    nodes: [
      {
        id: "asset-source",
        type: "source",
        title: "Asset Source",
        subtitle: "Unity data",
        outputs: [
          { id: "object", label: "Object", type: "Unity.Object" },
          { id: "path", label: "Path", type: "string" }
        ],
        parameters: [
          { id: "mode", label: "Mode", type: "select", value: "active", options: [
            { label: "Active", value: "active" },
            { label: "Pinned", value: "pinned" }
          ] }
        ]
      },
      {
        id: "process",
        type: "processor",
        title: "Process",
        subtitle: "Transform",
        inputs: [
          { id: "input", label: "Input", type: "Unity.Object" }
        ],
        outputs: [
          { id: "result", label: "Result", type: "object" }
        ],
        parameters: [
          { id: "enabled", label: "Enabled", type: "boolean", value: true },
          { id: "weight", label: "Weight", type: "number", value: 1, min: 0, max: 4, step: 0.1 }
        ]
      },
      {
        id: "apply",
        type: "output",
        title: "Apply",
        subtitle: "Write back",
        inputs: [
          { id: "value", label: "Value", type: "object" }
        ],
        parameters: [
          { id: "target", label: "Target", type: "string", value: "Configured asset" }
        ]
      }
    ],
    connections: [
      {
        id: "asset-source-process",
        from: { nodeId: "asset-source", portId: "object" },
        to: { nodeId: "process", portId: "input" }
      },
      {
        id: "process-apply",
        from: { nodeId: "process", portId: "result" },
        to: { nodeId: "apply", portId: "value" }
      }
    ]
  };
}

class TemplateGraphView extends GraphViewController {
  async loadGraph() {
    try {
      const response = await view.callScript("GraphViewApi", "Read", {});
      return response?.graph || fallbackGraph();
    } catch (error) {
      console.error("[node-graph] Read failed", error);
      return fallbackGraph();
    }
  }

  validateConnection(connection, graph) {
    const targetIsUsed = graph.connections.some((item) => {
      return item.to.nodeId === connection.to.nodeId
        && item.to.portId === connection.to.portId
        && item.id !== connection.id;
    });
    return targetIsUsed ? "Input port already has a connection." : true;
  }

  async saveGraph(graph) {
    await view.callScript("GraphViewApi", "Save", { graph });
  }

  async applyGraph(graph) {
    await view.callScript("GraphViewApi", "Apply", { graph });
  }
}

const graphView = defineGraphView(new TemplateGraphView());
</script>

<template>
  <GraphView :controller="graphView" title="" />
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
}
"#
    .to_string()
}

pub(super) fn view_api_cs() -> String {
    r#"using System;
using UnityEditor;

public static class GraphViewApi
{
    public static object Read()
    {
        return new
        {
            ok = true,
            message = "Ready",
            graph = DefaultGraph()
        };
    }

    public static object Save(string argsJson)
    {
        EditorPrefs.SetString("Locus.GraphViewApi.LastGraph", argsJson ?? "{}");
        return new
        {
            ok = true,
            message = "Saved"
        };
    }

    public static object Apply(string argsJson)
    {
        EditorPrefs.SetString("Locus.GraphViewApi.LastAppliedGraph", argsJson ?? "{}");
        return new
        {
            ok = true,
            message = "Applied"
        };
    }

    private static object DefaultGraph()
    {
        return new
        {
            layout = new { auto = "missing", direction = "right" },
            nodes = new object[]
            {
                new
                {
                    id = "asset-source",
                    type = "source",
                    title = "Asset Source",
                    subtitle = "Unity data",
                    outputs = new object[]
                    {
                        new { id = "object", label = "Object", type = "Unity.Object" },
                        new { id = "path", label = "Path", type = "string" }
                    },
                    parameters = new object[]
                    {
                        new
                        {
                            id = "mode",
                            label = "Mode",
                            type = "select",
                            value = "active",
                            options = new object[]
                            {
                                new { label = "Active", value = "active" },
                                new { label = "Pinned", value = "pinned" }
                            }
                        }
                    }
                },
                new
                {
                    id = "process",
                    type = "processor",
                    title = "Process",
                    subtitle = "Transform",
                    inputs = new object[]
                    {
                        new { id = "input", label = "Input", type = "Unity.Object" }
                    },
                    outputs = new object[]
                    {
                        new { id = "result", label = "Result", type = "object" }
                    },
                    parameters = new object[]
                    {
                        new { id = "enabled", label = "Enabled", type = "boolean", value = true },
                        new { id = "weight", label = "Weight", type = "number", value = 1.0, min = 0.0, max = 4.0, step = 0.1 }
                    }
                },
                new
                {
                    id = "apply",
                    type = "output",
                    title = "Apply",
                    subtitle = "Write back",
                    inputs = new object[]
                    {
                        new { id = "value", label = "Value", type = "object" }
                    },
                    parameters = new object[]
                    {
                        new { id = "target", label = "Target", type = "string", value = "Configured asset" }
                    }
                }
            },
            connections = new object[]
            {
                new
                {
                    id = "asset-source-process",
                    from = new { nodeId = "asset-source", portId = "object" },
                    to = new { nodeId = "process", portId = "input" }
                },
                new
                {
                    id = "process-apply",
                    from = new { nodeId = "process", portId = "result" },
                    to = new { nodeId = "apply", portId = "value" }
                }
            }
        };
    }
}
"#
    .to_string()
}
