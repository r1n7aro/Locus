import type { InjectedPromptItem } from "../../types";
import {
  estimatePromptTokens,
  estimateToolPrompt,
  toolMetaLoadMode,
} from "../agent/agentPromptDashboard";

const KNOWLEDGE_TOOL_NAMES = new Set([
  "knowledge_list",
  "knowledge_query",
  "knowledge_read",
  "knowledge_create",
  "knowledge_delete",
  "knowledge_move",
  "knowledge_edit",
  "skill_create",
  "skill_reload",
  "skill_list",
]);

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function toolNameFromItem(item: Pick<InjectedPromptItem, "title" | "meta">): string {
  const meta = asRecord(item.meta);
  const functionDef = asRecord(meta?.function);
  const functionName = functionDef?.name;
  return typeof functionName === "string" && functionName.trim()
    ? functionName.trim().toLowerCase()
    : item.title.trim().toLowerCase();
}

export function isKnowledgeInjectionItem(item: Pick<InjectedPromptItem, "id">): boolean {
  return item.id === "knowledge_context" || item.id.startsWith("knowledge_rule::");
}

export function isDirectKnowledgeToolItem(
  item: Pick<InjectedPromptItem, "kind" | "title" | "meta">,
): boolean {
  if (item.kind !== "tools") return false;
  return KNOWLEDGE_TOOL_NAMES.has(toolNameFromItem(item))
    && toolMetaLoadMode(item.meta) === "direct";
}

export function estimateTextTokens(text: string): number {
  return estimatePromptTokens(text.length);
}

export function estimateKnowledgeContextCostTokens(
  items: Array<Pick<InjectedPromptItem, "id" | "kind" | "title" | "content" | "meta">>,
): number {
  return items.reduce((total, item) => {
    if (isKnowledgeInjectionItem(item)) {
      return total + estimateTextTokens(item.content);
    }
    if (isDirectKnowledgeToolItem(item)) {
      return total + estimateToolPrompt(item.meta).tokens;
    }
    return total;
  }, 0);
}
