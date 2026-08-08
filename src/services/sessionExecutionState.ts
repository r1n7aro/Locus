import { emit } from "@tauri-apps/api/event";
import type { EffortLevel } from "../types";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const SESSION_EXECUTION_STATE_CHANGED_EVENT = "session-execution-state-changed";

export interface SessionExecutionStateChanged {
  sessionId: string;
  modelId: string;
  effort: EffortLevel;
}

export async function broadcastSessionExecutionState(
  payload: SessionExecutionStateChanged,
): Promise<void> {
  if (!hasTauriWindowRuntime() || !payload.sessionId.trim()) return;
  await emit(SESSION_EXECUTION_STATE_CHANGED_EVENT, {
    sessionId: payload.sessionId.trim(),
    modelId: payload.modelId.trim(),
    effort: payload.effort,
  });
}
