import type { ToolCallDisplay } from "../types";

/**
 * Parsing layer for the exit_plan_mode transcript block.
 *
 * exit_plan_mode carries no arguments — the plan travels in the tool OUTPUT
 * written by the backend (`execute_exit_plan_mode` in
 * src-tauri/src/agent/instance/mod.rs). These markers mirror the backend's
 * literal strings and are locked together by a static contract test, so a
 * backend wording change fails the suite instead of silently degrading the
 * block to the generic error rendering.
 */
export const APPROVED_PLAN_MARKER = "## Approved Plan:";
export const REJECTED_PLAN_PREFIX = "The user rejected the plan";
export const REJECTED_FEEDBACK_MARKER = "User feedback: ";

export type ExitPlanModeBlockState =
  | { kind: "awaiting" }
  | { kind: "approved"; plan: string }
  | { kind: "rejected"; feedback: string }
  | { kind: "error"; detail: string };

export function parseExitPlanModeBlock(
  toolCall: Pick<ToolCallDisplay, "status" | "output">,
): ExitPlanModeBlockState {
  if (toolCall.status === "running") return { kind: "awaiting" };

  const output = toolCall.output ?? "";

  const approvedIndex = output.indexOf(APPROVED_PLAN_MARKER);
  if (toolCall.status === "done" && approvedIndex >= 0) {
    return {
      kind: "approved",
      plan: output.slice(approvedIndex + APPROVED_PLAN_MARKER.length).trim(),
    };
  }

  if (output.startsWith(REJECTED_PLAN_PREFIX)) {
    const feedbackIndex = output.indexOf(REJECTED_FEEDBACK_MARKER);
    return {
      kind: "rejected",
      feedback:
        feedbackIndex >= 0
          ? output.slice(feedbackIndex + REJECTED_FEEDBACK_MARKER.length).trim()
          : "",
    };
  }

  return { kind: "error", detail: output };
}
