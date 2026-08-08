export interface ToolSearchToolSummary {
  name: string;
  description: string;
}

export interface ToolSearchOutputSummary {
  tools: ToolSearchToolSummary[];
}

const DESCRIPTION_SUMMARY_MAX_LENGTH = 240;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function summarizeToolSearchDescription(description: string): string {
  const firstParagraph = description
    .trim()
    .split(/\r?\n\s*\r?\n/, 1)[0]
    ?.replace(/\s+/g, " ")
    .trim() ?? "";

  if (firstParagraph.length <= DESCRIPTION_SUMMARY_MAX_LENGTH) {
    return firstParagraph;
  }
  return `${firstParagraph.slice(0, DESCRIPTION_SUMMARY_MAX_LENGTH - 1).trimEnd()}…`;
}

/**
 * Extracts the human-readable part of a Codex-native tool_search result.
 * Tool parameter schemas intentionally stay out of the transcript UI: the
 * exact wire name and the first description paragraph identify each match.
 */
export function parseToolSearchOutput(output: string): ToolSearchOutputSummary | null {
  try {
    const value: unknown = JSON.parse(output);
    if (!isRecord(value) || !Array.isArray(value.tools)) return null;

    const tools = value.tools.flatMap((tool): ToolSearchToolSummary[] => {
      if (!isRecord(tool) || typeof tool.name !== "string" || !tool.name.trim()) {
        return [];
      }
      return [{
        name: tool.name.trim(),
        description: typeof tool.description === "string"
          ? summarizeToolSearchDescription(tool.description)
          : "",
      }];
    });

    return { tools };
  } catch {
    return null;
  }
}
