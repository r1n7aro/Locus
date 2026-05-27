interface ViewToolCallOpenSource {
  name: string;
  arguments: string;
  output?: string;
  status?: string;
}

type JsonObject = Record<string, unknown>;

function parseJsonObject(text: string | undefined): JsonObject | null {
  if (!text) return null;
  try {
    const parsed = JSON.parse(text);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed as JsonObject
      : null;
  } catch {
    return null;
  }
}

function getString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function getNestedObject(value: unknown): JsonObject | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

function viewIdFromRunArgs(argumentsText: string): string {
  const args = parseJsonObject(argumentsText);
  if (!args) return "";
  return getString(args.viewId) || getString(args.view_id) || getString(args.id);
}

function viewIdFromRunOutput(output: string | undefined): string {
  const result = parseJsonObject(output);
  if (!result) return "";
  return getString(result.id) || getString(result.viewId) || getString(result.view_id);
}

function viewIdFromCreateOutput(output: string | undefined): string {
  const result = parseJsonObject(output);
  if (!result) return "";
  const summary = getNestedObject(result.summary);
  return getString(summary?.id) || getString(result.id) || getString(result.viewId);
}

function nonTemporaryCreateArgId(argumentsText: string): string {
  const args = parseJsonObject(argumentsText);
  if (!args || args.temporary === true) return "";
  return getString(args.id);
}

export function resolveViewToolOpenId(toolCall: ViewToolCallOpenSource): string {
  if (toolCall.status === "running") return "";

  if (toolCall.name === "view_run") {
    return viewIdFromRunArgs(toolCall.arguments) || viewIdFromRunOutput(toolCall.output);
  }

  if (toolCall.name === "view_create") {
    if (toolCall.status && toolCall.status !== "done") return "";
    return viewIdFromCreateOutput(toolCall.output) || nonTemporaryCreateArgId(toolCall.arguments);
  }

  return "";
}
