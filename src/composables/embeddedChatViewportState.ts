import type { SessionScrollState } from "./chatScrollState";

/** Stable viewport cache shared by editor instances that move between groups. */
export const embeddedChatViewportStates = new Map<string, SessionScrollState>();

const MAX_VIEWPORT_STATES = 128;

export function rememberEmbeddedChatViewportState(
  key: string,
  state: SessionScrollState,
): void {
  embeddedChatViewportStates.delete(key);
  embeddedChatViewportStates.set(key, state);
  if (embeddedChatViewportStates.size <= MAX_VIEWPORT_STATES) return;
  const oldest = embeddedChatViewportStates.keys().next().value as string | undefined;
  if (oldest && oldest !== key) embeddedChatViewportStates.delete(oldest);
}
