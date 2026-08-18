import type { ToolCallDisplay } from "../types";

const BACKGROUND_TOOL_MODES = new Set(["async", "notify", "async_notify"]);

export function isBackgroundToolCall(toolCall: Pick<ToolCallDisplay, "arguments">) {
  try {
    const args = JSON.parse(toolCall.arguments) as { async?: unknown };
    return typeof args.async === "string" && BACKGROUND_TOOL_MODES.has(args.async);
  } catch {
    return false;
  }
}

/**
 * A handoff starts only after activeToolCalls has been cleared, which closes
 * the foreground tool round. Normalize a transport that omitted toolCallDone
 * so the completed row cannot keep presenting a running spinner beside the
 * response-waiting indicator. Explicit background tools stay running until an
 * async task update settles them.
 */
export function settleToolCallDisplaysForHandoff(
  toolCalls: readonly ToolCallDisplay[],
): ToolCallDisplay[] {
  return toolCalls.map((toolCall) => ({
    id: toolCall.id,
    name: toolCall.name,
    arguments: toolCall.arguments,
    status: toolCall.status === "running" && !isBackgroundToolCall(toolCall)
      ? "done"
      : toolCall.status,
    order: toolCall.order,
    output: toolCall.output,
    images: toolCall.images ? [...toolCall.images] : undefined,
    progress: toolCall.progress ? { ...toolCall.progress } : toolCall.progress,
    nestedToolCalls: toolCall.nestedToolCalls
      ? settleToolCallDisplaysForHandoff(toolCall.nestedToolCalls)
      : undefined,
  }));
}

/**
 * A provider's provisional start contains only an id and name. If that start
 * is absent from the completed response it never receives arguments, output,
 * progress, or nested calls. Keep only materialized calls when promoting a
 * closed round into its completed handoff presentation.
 */
export function retainMaterializedToolCallDisplays(
  toolCalls: readonly ToolCallDisplay[],
): ToolCallDisplay[] {
  return toolCalls.flatMap((toolCall) => {
    const nestedToolCalls = toolCall.nestedToolCalls
      ? retainMaterializedToolCallDisplays(toolCall.nestedToolCalls)
      : undefined;
    const hasPayload = toolCall.arguments.trim().length > 0
      || toolCall.output !== undefined
      || !!toolCall.images?.length
      || toolCall.progress != null
      || !!nestedToolCalls?.length;
    if (!hasPayload) return [];
    return [{
      ...toolCall,
      nestedToolCalls,
    }];
  });
}
