export const CHAT_WORKSPACE_CONTENT_MAX_WIDTH = 980;

export function resolveChatContentBalanceInset(
  chatViewportWidth: number,
  sidebarWidth: number,
  contentMaxWidth = CHAT_WORKSPACE_CONTENT_MAX_WIDTH,
): number {
  const normalizedChatWidth = Math.max(0, Number.isFinite(chatViewportWidth) ? chatViewportWidth : 0);
  const normalizedSidebarWidth = Math.max(0, Number.isFinite(sidebarWidth) ? sidebarWidth : 0);
  const normalizedContentMaxWidth = Math.max(0, Number.isFinite(contentMaxWidth) ? contentMaxWidth : 0);
  if (normalizedSidebarWidth <= 0) return 0;
  return normalizedChatWidth - normalizedSidebarWidth >= normalizedContentMaxWidth
    ? normalizedSidebarWidth
    : 0;
}
