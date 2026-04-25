import type { Component } from "vue";
import UnityRunStatesToolBlock from "./UnityRunStatesToolBlock.vue";

const TOOL_BLOCK_OVERRIDES: Record<string, Component> = {
  unity_run_states: UnityRunStatesToolBlock,
};

export function resolveToolBlockOverride(toolName: string): Component | null {
  return TOOL_BLOCK_OVERRIDES[toolName] ?? null;
}
