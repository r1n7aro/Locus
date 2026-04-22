import { estimateToolPrompt } from "./agentPromptDashboard";

type JsonRecord = Record<string, unknown>;

export interface AgentToolParameterRow {
  path: string;
  depth: number;
  typeLabel: string;
  description: string;
  required: boolean;
  defaultValue: string | null;
  enumValues: string[];
}

export interface AgentToolDefinition {
  name: string;
  description: string;
  rawJson: string;
  schemaType: string;
  topLevelParameterCount: number;
  topLevelRequired: string[];
  parameterRows: AgentToolParameterRow[];
  promptCharCount: number;
  estimatedPromptTokens: number;
}

function asRecord(value: unknown): JsonRecord | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value as JsonRecord;
}

function readString(record: JsonRecord, key: string): string | null {
  const value = record[key];
  return typeof value === "string" ? value : null;
}

function readStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

function inferTypeLabel(schema: JsonRecord): string {
  const rawType = schema.type;
  if (Array.isArray(rawType)) {
    const types = rawType.filter((item): item is string => typeof item === "string");
    if (types.length > 0) return types.join(" | ");
  }
  if (typeof rawType === "string" && rawType.trim()) {
    return rawType;
  }
  if (asRecord(schema.properties)) return "object";
  if (schema.items !== undefined) return "array";
  if (Array.isArray(schema.enum) && schema.enum.length > 0) return "enum";
  return "any";
}

function formatInlineValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (value === null) return "null";
  try {
    const serialized = JSON.stringify(value);
    return serialized ?? String(value);
  } catch {
    return String(value);
  }
}

function makeParameterRow(
  path: string,
  depth: number,
  schema: JsonRecord,
  required: boolean,
): AgentToolParameterRow {
  const enumValues = Array.isArray(schema.enum)
    ? schema.enum.map((item) => formatInlineValue(item))
    : [];

  return {
    path,
    depth,
    typeLabel: inferTypeLabel(schema),
    description: readString(schema, "description") ?? "",
    required,
    defaultValue: schema.default === undefined ? null : formatInlineValue(schema.default),
    enumValues,
  };
}

function collectNestedRows(schema: JsonRecord, path: string, depth: number): AgentToolParameterRow[] {
  const rows: AgentToolParameterRow[] = [];

  const objectProperties = asRecord(schema.properties);
  if (objectProperties) {
    rows.push(...collectPropertyRows(schema, path, depth));
  }

  const items = asRecord(schema.items);
  if (items) {
    const itemPath = `${path}[]`;
    const hasNestedContent = !!asRecord(items.properties) || !!asRecord(items.items);
    const hasExplicitContent =
      !!readString(items, "description") ||
      items.type !== undefined ||
      items.default !== undefined ||
      (Array.isArray(items.enum) && items.enum.length > 0);

    if (hasNestedContent || hasExplicitContent) {
      rows.push(makeParameterRow(itemPath, depth, items, false));
    }

    rows.push(...collectNestedRows(items, itemPath, depth + 1));
  }

  return rows;
}

function collectPropertyRows(
  schema: JsonRecord,
  parentPath: string,
  depth: number,
): AgentToolParameterRow[] {
  const properties = asRecord(schema.properties);
  if (!properties) return [];

  const requiredFields = new Set(readStringArray(schema.required));
  const rows: AgentToolParameterRow[] = [];

  for (const [key, value] of Object.entries(properties)) {
    const child = asRecord(value);
    if (!child) continue;

    const path = parentPath ? `${parentPath}.${key}` : key;
    rows.push(makeParameterRow(path, depth, child, requiredFields.has(key)));
    rows.push(...collectNestedRows(child, path, depth + 1));
  }

  return rows;
}

export function parseAgentToolDefinition(meta: unknown): AgentToolDefinition | null {
  const raw = asRecord(meta);
  if (!raw) return null;

  const definition = asRecord(raw.function) ?? raw;
  const name = readString(definition, "name");
  if (!name) return null;

  const parameters = asRecord(definition.parameters);
  const topLevelProperties = asRecord(parameters?.properties);
  const promptEstimate = estimateToolPrompt(definition);

  return {
    name,
    description: readString(definition, "description") ?? "",
    rawJson: JSON.stringify(raw, null, 2),
    schemaType: parameters ? inferTypeLabel(parameters) : "any",
    topLevelParameterCount: topLevelProperties ? Object.keys(topLevelProperties).length : 0,
    topLevelRequired: parameters ? readStringArray(parameters.required) : [],
    parameterRows: parameters ? collectPropertyRows(parameters, "", 0) : [],
    promptCharCount: promptEstimate.chars,
    estimatedPromptTokens: promptEstimate.tokens,
  };
}
