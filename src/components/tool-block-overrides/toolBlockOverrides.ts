import type { Component } from "vue";
import BlenderExecuteToolBlock from "./BlenderExecuteToolBlock.vue";
import ExitPlanModeToolBlock from "./ExitPlanModeToolBlock.vue";
import KnowledgeQueryToolBlock from "./KnowledgeQueryToolBlock.vue";
import UnityExecuteToolBlock from "./UnityExecuteToolBlock.vue";
import UnityRunStatesToolBlock from "./UnityRunStatesToolBlock.vue";
import UnityTestRunToolBlock from "./UnityTestRunToolBlock.vue";

const TOOL_BLOCK_OVERRIDES: Record<string, Component> = {
  blender_execute: BlenderExecuteToolBlock,
  exit_plan_mode: ExitPlanModeToolBlock,
  knowledge_query: KnowledgeQueryToolBlock,
  unity_execute: UnityExecuteToolBlock,
  unity_run_states: UnityRunStatesToolBlock,
  unity_test_run: UnityTestRunToolBlock,
};

export function resolveToolBlockOverride(toolName: string): Component | null {
  return TOOL_BLOCK_OVERRIDES[toolName] ?? null;
}
