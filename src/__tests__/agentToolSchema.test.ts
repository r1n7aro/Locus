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

  it("returns directly readable physical locations through knowledge_query", () => {
    const raw = readFileSync(resolve(cwd, "tools/knowledge_query.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "knowledge_query",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.additionalProperties).toBe(false);
    expect(definition.description).toContain("real file path");
    expect(definition.description).toContain("physical line range");
    expect(definition.description).toContain("Use `read`");
    expect(definition.description).toContain("titles are omitted");
    expect(definition.parameters.properties.includeSummary.default).toBe(false);
    expect(definition.parameters.properties.includeHitContext.default).toBe(true);
    expect(definition.parameters.properties.hitContextMaxChars).toMatchObject({
      default: 220,
      minimum: 80,
      maximum: 1000,
    });
  });

  it("documents read outline mode as an opt-in C# and Markdown outline", () => {
    const raw = readFileSync(resolve(cwd, "tools/read.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "read",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.properties.outline).toMatchObject({
      type: "boolean",
      default: false,
    });
    expect(definition.description).toContain("C# (.cs)");
    expect(definition.description).toContain("Markdown (.md)");
    expect(definition.description).toContain("instead of original file content");
    expect(definition.description).toContain("unsupported file types return an error");
  });

  it("exposes one atomic same-file edit batch with original-snapshot semantics", () => {
    const raw = readFileSync(resolve(cwd, "tools/edit.json"), "utf8");
    const definition = JSON.parse(raw);
    const devToolUsageRule = readFileSync(
      resolve(cwd, "agent/dev/rule/tool_usage_strategy.md"),
      "utf8",
    );
    const tool = parseAgentToolDefinition({
      name: "edit",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.additionalProperties).toBe(false);
    expect(tool?.topLevelRequired).toEqual(["filePath", "edits"]);
    expect(tool?.parameterRows.map((row) => row.path)).toEqual([
      "filePath",
      "edits",
      "edits[]",
      "edits[].oldString",
      "edits[].newString",
      "edits[].replaceAll",
    ]);
    expect(definition.parameters.properties.edits.minItems).toBe(1);
    expect(definition.parameters.properties.edits.items.additionalProperties).toBe(false);
    expect(tool?.parameterRows.find((row) => row.path === "edits[].replaceAll")?.defaultValue).toBe("false");
    expect(definition.description).toContain("matched against the original file content");
    expect(definition.description).toContain("Array order does not make edits sequential");
    expect(definition.description).toContain("entire call fails without changing the file");
    expect(devToolUsageRule).toContain("call's `edits` array");
    expect(devToolUsageRule).toContain("matched against the same original file");
    expect(devToolUsageRule).toContain("Array order does not create dependencies");
  });

  it("documents automatic knowledge frontmatter in write", () => {
    const raw = readFileSync(resolve(cwd, "tools/write.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "write",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(tool?.topLevelRequired).toEqual(["filePath", "content"]);
    expect(definition.description).toContain("provide Markdown body content only");
    expect(definition.description).toContain("reports every generated field");
  });

  it("keeps create_skill_package package-only", () => {
    const raw = readFileSync(resolve(cwd, "tools/create_skill_package.json"), "utf8");
    const definition = JSON.parse(raw);
    const tool = parseAgentToolDefinition({
      name: "create_skill_package",
      ...definition,
    });

    expect(tool).not.toBeNull();
    expect(definition.parameters.additionalProperties).toBe(false);
    expect(tool?.topLevelRequired).toEqual(["name", "version", "summary"]);
    expect(tool?.parameterRows.map((row) => row.path)).not.toContain("kind");
    expect(tool?.parameterRows.map((row) => row.path)).not.toContain("path");
    expect(tool?.parameterRows.map((row) => row.path)).not.toContain("tools");
    expect(definition.description).toContain("app Skill package");
    expect(definition.parameters.properties.body.description).not.toContain("L1");
    expect(definition.parameters.properties.summary.description).toContain("frontmatter `summary`");
  });

  it("keeps skill_reload sources aligned with backend resolution", () => {
    const definition = JSON.parse(
      readFileSync(resolve(cwd, "tools/skill_reload.json"), "utf8"),
    );

    expect(definition.parameters.additionalProperties).toBe(false);
    expect(definition.parameters.required).toEqual(["name"]);
    expect(definition.parameters.properties.source.enum).toEqual([
      "project",
      "app",
      "pluginApp",
      "pluginProject",
      "externalUser",
      "externalProject",
    ]);

    const listDefinition = JSON.parse(
      readFileSync(resolve(cwd, "tools/skill_list.json"), "utf8"),
    );
    expect(listDefinition.parameters.properties.source.enum).toEqual(
      definition.parameters.properties.source.enum,
    );
  });
});
