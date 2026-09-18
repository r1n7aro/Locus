import { watch, watchPostEffect } from "vue";
import { isSessionUnread, markSessionAttentionRead, sessionAttention } from "../services/sessionAttention";

/** Read the loaded, active session in a focused window, regardless of its scroll position. */
export function useSessionAttentionReader(
  sessionId: () => string | null | undefined,
  scrollElement: () => HTMLElement | null,
  canReadSession: () => boolean,
): void {
  const saving = new Set<string>();
  function check() {
    const id = sessionId();
    const element = scrollElement();
    const doc = element?.ownerDocument;
    const entry = id ? sessionAttention.value.sessions[id] : undefined;
    if (!id || !entry || !isSessionUnread(id)) return;
    // Track tab activation and loading even while the window is unfocused.
    const ready = canReadSession();
    if (!ready || !element || !doc?.hasFocus()
      || doc.visibilityState !== "visible" || !element.getClientRects().length || element.clientHeight <= 0
    ) return;
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
    doc.addEventListener("visibilitychange", check);
    doc.addEventListener("focusin", check);
    ownerWindow?.addEventListener("focus", check);
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(check);
    resize?.observe(element);
    const mutation = new MutationObserver(check);
    mutation.observe(element, { childList: true, subtree: true, attributes: true,
      attributeFilter: ["class", "style", "data-chat-message-id"] });
    onCleanup(() => {
      doc.removeEventListener("visibilitychange", check);
      doc.removeEventListener("focusin", check);
      ownerWindow?.removeEventListener("focus", check);
      resize?.disconnect();
      mutation.disconnect();
    });
  }, { immediate: true, flush: "post" });
}
