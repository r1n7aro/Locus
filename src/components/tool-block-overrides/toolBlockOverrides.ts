import type { Component } from "vue";
import ExitPlanModeToolBlock from "./ExitPlanModeToolBlock.vue";
import KnowledgeQueryToolBlock from "./KnowledgeQueryToolBlock.vue";
import UnityExecuteToolBlock from "./UnityExecuteToolBlock.vue";
import UnityRunStatesToolBlock from "./UnityRunStatesToolBlock.vue";
import UnityTestToolBlock from "./UnityTestToolBlock.vue";

const TOOL_BLOCK_OVERRIDES: Record<string, Component> = {
  exit_plan_mode: ExitPlanModeToolBlock,
  knowledge_query: KnowledgeQueryToolBlock,
  unity_execute: UnityExecuteToolBlock,
  unity_run_states: UnityRunStatesToolBlock,
  unity_test_run: UnityTestToolBlock,
};

export function resolveToolBlockOverride(toolName: string): Component | null {
  return TOOL_BLOCK_OVERRIDES[toolName] ?? null;
}
