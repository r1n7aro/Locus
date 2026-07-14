import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseAgentToolDefinition } from "../components/agent/toolSchema";

const cwd = process.cwd();

describe("parseAgentToolDefinition", () => {
  it("reads top-level required fields from a tool definition", () => {
    const meta = {
      name: "read",
      description: "Read a file",
      parameters: {
        type: "object",
        properties: {
          filePath: {
            type: "string",
            description: "Path to file",
          },
          offset: {
            type: "integer",
          },
        },
        required: ["filePath"],
      },
    };
    const tool = parseAgentToolDefinition(meta);
    const expectedChars = JSON.stringify({
      type: "function",
      function: meta,
    }).length;

    expect(tool).not.toBeNull();
    expect(tool?.topLevelParameterCount).toBe(2);
    expect(tool?.topLevelRequired).toEqual(["filePath"]);
    expect(tool?.parameterRows.map((row) => row.path)).toEqual(["filePath", "offset"]);
    expect(tool?.parameterRows.find((row) => row.path === "filePath")?.required).toBe(true);
    expect(tool?.promptCharCount).toBe(expectedChars);
    expect(tool?.estimatedPromptTokens).toBe(Math.ceil(expectedChars / 4) + 32);
  });

  it("flattens nested object and array schema paths", () => {
    const tool = parseAgentToolDefinition({
      function: {
        name: "nested_tool",
        description: "Nested tool",
        parameters: {
          type: "object",
          properties: {
            spec: {
              type: "object",
              properties: {
                nodes: {
                  type: "array",
                  items: {
                    type: "object",
                    properties: {
                      id: { type: "string" },
                      update: {
                        type: "object",
                        properties: {
                          mode: {
                            type: "string",
                            enum: ["serialized", "code"],
                          },
                        },
                        required: ["mode"],
                      },
                    },
                    required: ["id"],
                  },
                },
              },
              required: ["nodes"],
            },
          },
          required: ["spec"],
        },
      },
    });

    expect(tool).not.toBeNull();
    expect(tool?.parameterRows.map((row) => row.path)).toEqual([
      "spec",
      "spec.nodes",
      "spec.nodes[]",
      "spec.nodes[].id",
      "spec.nodes[].update",
      "spec.nodes[].update.mode",
    ]);
    expect(tool?.parameterRows.find((row) => row.path === "spec.nodes[].update.mode")?.required).toBe(true);
    expect(tool?.parameterRows.find((row) => row.path === "spec.nodes[].update.mode")?.enumValues).toEqual([
      "serialized",
      "code",
    ]);
  });

  it("exposes exact knowledge sections through knowledge_read", () => {
    const raw = readFileSync(resolve(cwd, "tools/knowledge_read.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "knowledge_read",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.additionalProperties).toBe(false);
    expect(tool?.topLevelRequired).toEqual(["path"]);
    expect(tool?.parameterRows.some((row) => row.path === "kind")).toBe(false);
    expect(tool?.parameterRows.find((row) => row.path === "part")?.enumValues).toEqual([
      "full",
      "summary",
      "body",
      "maintenanceRules",
    ]);
    expect(tool?.parameterRows.find((row) => row.path === "part")?.defaultValue).toBe("full");
    expect(definition.description).toContain("exactly the stored text");
    expect(definition.description).toContain("no heading, trimming, placeholder, added newline, or runtime annotation");
  });

  it("keeps edit limited to one public file edit", () => {
    const raw = readFileSync(resolve(cwd, "tools/edit.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "edit",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.additionalProperties).toBe(false);
    expect(tool?.topLevelRequired).toEqual(["filePath", "oldString", "newString"]);
    expect(tool?.parameterRows.map((row) => row.path)).toEqual([
      "filePath",
      "oldString",
      "newString",
      "replaceAll",
    ]);
    expect(tool?.parameterRows.some((row) => row.path === "edits")).toBe(false);
    expect(tool?.parameterRows.find((row) => row.path === "replaceAll")?.defaultValue).toBe("false");
  });

  it("keeps knowledge_edit limited to one targeted document edit", () => {
    const raw = readFileSync(resolve(cwd, "tools/knowledge_edit.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "knowledge_edit",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.additionalProperties).toBe(false);
    expect(tool?.topLevelRequired).toEqual(["path", "section", "oldString", "newString"]);
    expect(tool?.parameterRows.map((row) => row.path)).toEqual([
      "path",
      "section",
      "oldString",
      "newString",
    ]);
    expect(tool?.parameterRows.every((row) => row.required)).toBe(true);
    expect(tool?.parameterRows.find((row) => row.path === "section")?.enumValues).toEqual([
      "summary",
      "body",
      "maintenanceRules",
    ]);
    expect(definition.description).toContain("`part` equal to `section`");
    expect(definition.description).toContain("`full` is a rendered reading view");
  });

  it("keeps knowledge_create free of metadata and directory config patches", () => {
    const raw = readFileSync(resolve(cwd, "tools/knowledge_create.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "knowledge_create",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.additionalProperties).toBe(false);
    expect(definition.parameters.properties.document.additionalProperties).toBe(false);
    expect(tool?.parameterRows.some((row) => row.path === "config")).toBe(false);
    expect(tool?.parameterRows.some((row) => row.path === "document.title")).toBe(false);
    expect(tool?.parameterRows.some((row) => row.path === "document.injectMode")).toBe(false);
    expect(tool?.parameterRows).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ path: "kind" }),
        expect.objectContaining({ path: "path" }),
        expect.objectContaining({ path: "document.summary" }),
        expect.objectContaining({ path: "document.body" }),
        expect.objectContaining({ path: "document.maintenanceRules" }),
      ]),
    );
  });
});
