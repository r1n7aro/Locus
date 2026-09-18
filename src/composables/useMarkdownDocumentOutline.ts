import { EditorView } from "@codemirror/view";
import { computed, nextTick, onUnmounted, ref, watch, type Ref } from "vue";
import {
  extractKnowledgeDocumentOutline,
  type KnowledgeDocumentOutlineItem,
} from "../components/knowledge/knowledgeDocumentOutline";

const DOCUMENT_OUTLINE_STICKY_TOP = 40;
const DOCUMENT_OUTLINE_BODY_LEAD = 16;
const DOCUMENT_OUTLINE_BOTTOM_GUTTER = 24;

type MarkdownDocumentOutlineOptions = {
  documentKey: () => string;
  source: () => string;
  active: () => boolean;
  scroller: Ref<HTMLElement | null>;
  page: Ref<HTMLElement | null>;
  body: Ref<HTMLElement | null>;
  editor: Ref<{ getEditorView: () => EditorView | null } | null>;
};

export function useMarkdownDocumentOutline(options: MarkdownDocumentOutlineOptions) {
  const activeOutlineId = ref("");
  const outlineViewportHeight = ref(0);
  const outlineViewportTop = ref(DOCUMENT_OUTLINE_STICKY_TOP);
  const outlineStartOffset = ref(DOCUMENT_OUTLINE_STICKY_TOP);
  let outlineUpdateFrame = 0;
  let outlineRefreshTimer: ReturnType<typeof setTimeout> | null = null;
  let outlineResizeObserver: ResizeObserver | null = null;

  const documentOutlineItems = ref<KnowledgeDocumentOutlineItem[]>([]);
  const documentOutlineBaseLevel = computed(() =>
    documentOutlineItems.value.reduce(
      (lowest, item) => Math.min(lowest, item.level),
      6,
    )
  );
  const documentOutlineMarginTop = computed(() => `${outlineStartOffset.value}px`);
  const documentOutlineMaxHeight = computed(() => (
    outlineViewportHeight.value > 0
      ? `${Math.max(
          160,
          outlineViewportHeight.value
            - outlineViewportTop.value
            - DOCUMENT_OUTLINE_BOTTOM_GUTTER,
        )}px`
      : undefined
  ));

  function outlineItemPadding(item: KnowledgeDocumentOutlineItem): string {
    const depth = Math.max(0, item.level - documentOutlineBaseLevel.value);
    return `${8 + Math.min(depth, 4) * 12}px`;
  }

  function outlineItemScreenTop(
    view: EditorView,
    item: KnowledgeDocumentOutlineItem,
  ): number {
    const position = Math.min(view.state.doc.length, Math.max(0, item.from));
    return view.documentTop + view.lineBlockAt(position).top;
  }

  function updateDocumentOutlineActive(): void {
    outlineUpdateFrame = 0;
    updateDocumentOutlineLayout();
    const items = documentOutlineItems.value;
    if (!items.length) {
      activeOutlineId.value = "";
      return;
    }

    const scroller = options.scroller.value;
    const view = options.editor.value?.getEditorView();
    if (!scroller || !view) {
      activeOutlineId.value = items[0]?.id ?? "";
      return;
    }

    if (scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2) {
      activeOutlineId.value = items[items.length - 1]?.id ?? "";
      return;
    }

    const activationLine = scroller.getBoundingClientRect().top + 48;
    let activeItem = items[0];
    for (const item of items) {
      if (outlineItemScreenTop(view, item) > activationLine) break;
      activeItem = item;
    }
    activeOutlineId.value = activeItem?.id ?? "";
  }

  function scheduleDocumentOutlineActiveUpdate(): void {
    if (outlineUpdateFrame) return;
    outlineUpdateFrame = requestAnimationFrame(updateDocumentOutlineActive);
  }

  function scrollToDocumentOutlineItem(item: KnowledgeDocumentOutlineItem): void {
    const scroller = options.scroller.value;
    const view = options.editor.value?.getEditorView();
    if (!scroller || !view) return;

    activeOutlineId.value = item.id;
    // Let CodeMirror resolve virtualized headings and remeasure intervening
    // tables/images before scrolling the outer document to the final position.
    view.dispatch({
      effects: EditorView.scrollIntoView(
        Math.min(view.state.doc.length, Math.max(0, item.from)),
        { y: "start", yMargin: 28 },
      ),
    });
  }

  function updateDocumentOutlineLayout(): void {
    const scroller = options.scroller.value;
    const page = options.page.value;
    const body = options.body.value;
    outlineViewportHeight.value = scroller?.clientHeight ?? 0;
    if (!scroller || !page || !body) {
      outlineViewportTop.value = DOCUMENT_OUTLINE_STICKY_TOP;
      outlineStartOffset.value = DOCUMENT_OUTLINE_STICKY_TOP;
      return;
    }

    const scrollerRect = scroller.getBoundingClientRect();
    const pageRect = page.getBoundingClientRect();
    const bodyRect = body.getBoundingClientRect();
    outlineStartOffset.value = Math.max(
      DOCUMENT_OUTLINE_STICKY_TOP,
      Math.round(bodyRect.top - pageRect.top - DOCUMENT_OUTLINE_BODY_LEAD),
    );
    outlineViewportTop.value = Math.max(
      DOCUMENT_OUTLINE_STICKY_TOP,
      Math.round(bodyRect.top - scrollerRect.top - DOCUMENT_OUTLINE_BODY_LEAD),
    );
  }

  function observeDocumentOutlineLayout(
    scroller: HTMLElement | null,
    page: HTMLElement | null,
    body: HTMLElement | null,
  ): void {
    outlineResizeObserver?.disconnect();
    outlineResizeObserver = null;
    updateDocumentOutlineLayout();
    if (!scroller || typeof ResizeObserver === "undefined") return;
    outlineResizeObserver = new ResizeObserver(() => {
      scheduleDocumentOutlineActiveUpdate();
    });
    outlineResizeObserver.observe(scroller);
    if (page) outlineResizeObserver.observe(page);
    if (body) outlineResizeObserver.observe(body);
  }

  watch(
    [options.scroller, options.page, options.body],
    ([scroller, page, body]) => observeDocumentOutlineLayout(scroller, page, body),
    { flush: "post" },
  );

  function applyDocumentOutlineSource(source: string): void {
    outlineRefreshTimer = null;
    documentOutlineItems.value = extractKnowledgeDocumentOutline(source);
    if (!documentOutlineItems.value.some((item) => item.id === activeOutlineId.value)) {
      activeOutlineId.value = documentOutlineItems.value[0]?.id ?? "";
    }
    void nextTick(scheduleDocumentOutlineActiveUpdate);
  }

  watch(
    () => [options.documentKey(), options.source()] as const,
    ([documentKey, source], previous) => {
      if (outlineRefreshTimer !== null) clearTimeout(outlineRefreshTimer);
      const documentChanged = !previous || previous[0] !== documentKey;
      if (documentChanged) {
        applyDocumentOutlineSource(source);
        return;
      }
      outlineRefreshTimer = setTimeout(() => applyDocumentOutlineSource(source), 120);
    },
    { flush: "post", immediate: true },
  );
  watch(
    () => [options.documentKey(), options.active()] as const,
    () => void nextTick(scheduleDocumentOutlineActiveUpdate),
    { flush: "post" },
  );

  onUnmounted(() => {
    if (outlineUpdateFrame) cancelAnimationFrame(outlineUpdateFrame);
    if (outlineRefreshTimer !== null) clearTimeout(outlineRefreshTimer);
    outlineResizeObserver?.disconnect();
  });

  return {
    documentOutlineItems,
    activeOutlineId,
    documentOutlineMarginTop,
    documentOutlineMaxHeight,
    outlineItemPadding,
    scrollToDocumentOutlineItem,
    scheduleDocumentOutlineActiveUpdate,
  };
}
