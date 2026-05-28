import type { Component } from "vue";
import KnowledgeQueryToolBlock from "./KnowledgeQueryToolBlock.vue";
import UnityExecuteToolBlock from "./UnityExecuteToolBlock.vue";
import UnityRunStatesToolBlock from "./UnityRunStatesToolBlock.vue";

const TOOL_BLOCK_OVERRIDES: Record<string, Component> = {
  knowledge_query: KnowledgeQueryToolBlock,
  unity_execute: UnityExecuteToolBlock,
  unity_run_states: UnityRunStatesToolBlock,
};

export function resolveToolBlockOverride(toolName: string): Component | null {
  return TOOL_BLOCK_OVERRIDES[toolName] ?? null;
}
