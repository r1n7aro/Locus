import { watch, watchPostEffect } from "vue";
import { isSessionUnread, markSessionAttentionRead, sessionAttention, type SessionAttentionEntry } from "../services/sessionAttention";

export function sessionResultIsVisible(scrollElement: HTMLElement | null, messageId: string | undefined): boolean {
  if (!scrollElement || !messageId) return false;
  const viewport = scrollElement.getBoundingClientRect();
  return Array.from(scrollElement.querySelectorAll<HTMLElement>("[data-chat-message-id]")).some((element) => {
    if (element.dataset.chatMessageId !== messageId) return false;
    const bounds = element.getBoundingClientRect();
    return bounds.width > 0 && bounds.height > 0 && bounds.bottom > viewport.top && bounds.top < viewport.bottom;
  });
}

/** Mounted/selected is not read: the rendered result must be visible in a focused window. */
export function useSessionAttentionReader(
  sessionId: () => string | null | undefined,
  scrollElement: () => HTMLElement | null,
  resultRendered: (entry: SessionAttentionEntry) => boolean,
): void {
  const saving = new Set<string>();
  function check() {
    const id = sessionId();
    const element = scrollElement();
    const doc = element?.ownerDocument;
    const entry = id ? sessionAttention.value.sessions[id] : undefined;
    if (!id || !entry || !isSessionUnread(id)) return;
    // Track render readiness even while focus or scroll position prevents reading.
    const rendered = resultRendered(entry);
    if (!element || !doc?.hasFocus()
      || doc.visibilityState !== "visible" || !element.getClientRects().length || element.clientHeight <= 0
      || element.scrollHeight - element.scrollTop - element.clientHeight > 32 || !rendered) return;
    const key = `${id}:${entry.sequence}`;
    if (saving.has(key)) return;
    saving.add(key);
    void markSessionAttentionRead(id, entry.sequence)
      .catch((error) => console.warn("[sessionAttention] Could not mark result read", error))
      .finally(() => saving.delete(key));
  }

  watchPostEffect(check);
  watch(scrollElement, (element, _previous, onCleanup) => {
    if (!element) return;
    const doc = element.ownerDocument;
    const ownerWindow = doc.defaultView;
    element.addEventListener("scroll", check, { passive: true });
    doc.addEventListener("visibilitychange", check);
    ownerWindow?.addEventListener("focus", check);
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(check);
    resize?.observe(element);
    const mutation = new MutationObserver(check);
    mutation.observe(element, { childList: true, subtree: true, attributes: true,
      attributeFilter: ["class", "style", "data-chat-message-id"] });
    onCleanup(() => {
      element.removeEventListener("scroll", check);
      doc.removeEventListener("visibilitychange", check);
      ownerWindow?.removeEventListener("focus", check);
      resize?.disconnect();
      mutation.disconnect();
    });
  }, { immediate: true, flush: "post" });
}
