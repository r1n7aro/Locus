import type { ToolCallDisplay, ToolCallInfo } from "../types";

export interface SkillLoadedMarker {
  name: string;
  path: string;
}

interface KnowledgeReadArguments {
  path?: unknown;
  filePath?: unknown;
  part?: unknown;
}

function parseKnowledgeReadArguments(argumentsText: string): KnowledgeReadArguments | null {
  try {
    const parsed = JSON.parse(argumentsText);
    return parsed && typeof parsed === "object" ? parsed as KnowledgeReadArguments : null;
  } catch {
    return null;
  }
}

function normalizeSkillPath(path: unknown): string | null {
  if (typeof path !== "string") return null;
  const normalized = path.trim().replace(/\\/g, "/");
  const relative = normalized.replace(/^\/+/, "");
  if (relative.toLowerCase().startsWith("skill/")) return relative;
  const workspaceMarker = "/locus/knowledge/skill/";
  const workspaceIndex = `/${relative.toLowerCase()}`.indexOf(workspaceMarker);
  if (workspaceIndex >= 0) {
    return `skill/${relative.slice(workspaceIndex + workspaceMarker.length - 1)}`;
  }
  if (/\/skills\/[^/]+\/skill\.md$/i.test(normalized)) return normalized;
  return null;
}

function readPartLoadsSkillContent(part: unknown): boolean {
  return part === undefined || part === null || part === "" || part === "full" || part === "body";
}

function cleanHeadingText(value: string): string {
  return value
    .replace(/^#+\s*/, "")
    .replace(/\s+#+\s*$/, "")
    .trim();
}

function titleFromKnowledgeReadOutput(output: string | undefined): string | null {
  if (!output) return null;
  for (const line of output.split(/\r?\n/)) {
    const trimmed = line.replace(/^\s*\d+\t/, "").trim();
    if (!trimmed.startsWith("# ") || trimmed.startsWith("## ")) continue;
    const title = cleanHeadingText(trimmed);
    if (title) return title;
  }
  return null;
}

function titleFromSkillPath(path: string): string {
  const segments = path.split("/").filter(Boolean);
  const lastSegment = segments[segments.length - 1] ?? "";
  const fallbackSegment =
    lastSegment.toLowerCase() === "skill.md" && segments.length > 1
      ? segments[segments.length - 2]
      : lastSegment;
  const withoutExtension = fallbackSegment.replace(/\.md$/i, "").trim();
  return withoutExtension || "Skill";
}

type SkillLoadedToolCallSource = Pick<ToolCallInfo, "id" | "name" | "arguments" | "outcome">
  | Pick<ToolCallDisplay, "id" | "name" | "arguments" | "status">;

function isCompletedSkillRead(toolCall: SkillLoadedToolCallSource): boolean {
  const outcome = "outcome" in toolCall ? toolCall.outcome : undefined;
  const status = "status" in toolCall ? toolCall.status : undefined;
  return (
    (toolCall.name === "read" || toolCall.name === "knowledge_read")
    && outcome !== "error"
    && outcome !== "interrupted"
    && status !== "running"
    && status !== "error"
    && status !== "interrupted"
  );
}

export function resolveSkillLoadedMarkerForToolCall(
  toolCall: SkillLoadedToolCallSource,
  output?: string,
): SkillLoadedMarker | null {
  if (!isCompletedSkillRead(toolCall)) return null;
  const args = parseKnowledgeReadArguments(toolCall.arguments);
  const path = normalizeSkillPath(args?.filePath ?? args?.path);
  if (!path || !readPartLoadsSkillContent(args?.part)) return null;
  return {
    name: titleFromKnowledgeReadOutput(output) ?? titleFromSkillPath(path),
    path,
  };
}
