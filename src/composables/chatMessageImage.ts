import { invoke } from "@tauri-apps/api/core";
import { domToBlob } from "modern-screenshot";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";

/** Space around the message content, in CSS pixels. */
const IMAGE_PADDING = 16;
const MAX_IMAGE_SCALE = 2;
/** Chromium rejects canvases with an edge longer than this. */
const MAX_IMAGE_EDGE = 32_767;
/** Keeps the canvas, the PNG encoder and the native DIB copy within a few hundred MB. */
export const MAX_CHAT_MESSAGE_IMAGE_PIXELS = 33_554_432;

export class ChatMessageImageTooLargeError extends Error {
  constructor() {
    super("Message is too large to render as an image");
    this.name = "ChatMessageImageTooLargeError";
  }
}

function cssEscape(value: string, ownerWindow: Window) {
  const css = (ownerWindow as Window & { CSS?: typeof CSS }).CSS;
  return typeof css?.escape === "function" ? css.escape(value) : value.replace(/["\\]/g, "\\$&");
}

/**
 * The rendered block that the message context menu highlights: one item stack
 * per user message, one per assistant group (anchored on the group's last message).
 */
export function findChatMessageImageTarget(root: HTMLElement, messageId: string): HTMLElement | null {
  const ownerWindow = root.ownerDocument.defaultView ?? window;
  const id = cssEscape(messageId, ownerWindow);
  return root.querySelector<HTMLElement>(`.chat-transcript-item-stack[data-scroll-anchor-id="${id}"]`)
    ?? root.querySelector<HTMLElement>(`[data-chat-message-id="${id}"]`);
}

/** Device-pixel scale for the image, or null when even 1x exceeds the canvas limits. */
export function chatMessageImageScale(width: number, height: number, devicePixelRatio: number): number | null {
  if (!(width > 0 && height > 0)) return null;
  const preferred = Math.min(
    MAX_IMAGE_SCALE,
    Math.max(1, Number.isFinite(devicePixelRatio) ? devicePixelRatio : 1),
  );
  const scale = Math.min(
    preferred,
    MAX_IMAGE_EDGE / width,
    MAX_IMAGE_EDGE / height,
    Math.sqrt(MAX_CHAT_MESSAGE_IMAGE_PIXELS / (width * height)),
  );
  return scale >= 1 ? scale : null;
}

/** The first opaque background behind the element, so the image matches the visible surface. */
function surfaceColorBehind(element: HTMLElement): string {
  const ownerDocument = element.ownerDocument;
  const ownerWindow = ownerDocument.defaultView ?? window;
  const probe = ownerDocument.createElement("canvas");
  probe.width = 1;
  probe.height = 1;
  const context = probe.getContext("2d", { willReadFrequently: true });
  const isOpaque = (color: string) => {
    if (!context || !color) return false;
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = "rgba(0, 0, 0, 0)";
    context.fillStyle = color;
    context.fillRect(0, 0, 1, 1);
    return context.getImageData(0, 0, 1, 1).data[3] === 255;
  };
  for (let node = element.parentElement; node; node = node.parentElement) {
    const color = ownerWindow.getComputedStyle(node).backgroundColor;
    if (isOpaque(color)) return color;
  }
  const token = ownerWindow.getComputedStyle(ownerDocument.documentElement)
    .getPropertyValue("--bg-color")
    .trim();
  return isOpaque(token) ? token : "Canvas";
}

const XHTML_NAMESPACE = "http://www.w3.org/1999/xhtml";
/** Elements the renderer does not clone. */
const UNCLONED_TAGS = new Set(["STYLE", "SCRIPT"]);
/** Replaced elements keep their measured size. */
const FIXED_SIZE_TAGS = new Set(["IMG", "CANVAS", "VIDEO", "IFRAME", "INPUT", "TEXTAREA", "SELECT"]);

function hasAutoHeight(element: Element) {
  if (element.namespaceURI !== XHTML_NAMESPACE || FIXED_SIZE_TAGS.has(element.tagName)) return false;
  if (typeof element.computedStyleMap !== "function") return false;
  try {
    return String(element.computedStyleMap().get("height")) === "auto";
  } catch {
    return false;
  }
}

/**
 * The renderer pins every clone to its measured pixel height. A parent with an
 * explicit height no longer lets its last child's bottom margin collapse
 * through it, which drops the spacing between Markdown blocks. Elements whose
 * height is `auto` in the transcript get it back so the clone lays out the
 * same way. The clone mirrors the source tree minus style/script elements; a
 * subtree whose shape differs keeps the pinned heights.
 */
function releaseAutoHeights(source: Element, clone: Element) {
  const sourceChildren = Array.from(source.children).filter((child) => !UNCLONED_TAGS.has(child.tagName));
  const cloneChildren = Array.from(clone.children);
  if (sourceChildren.length !== cloneChildren.length) return;
  sourceChildren.forEach((sourceChild, index) => {
    const cloneChild = cloneChildren[index];
    if (sourceChild.namespaceURI !== XHTML_NAMESPACE) return;
    if (hasAutoHeight(sourceChild) && "style" in cloneChild) {
      const style = (cloneChild as HTMLElement).style;
      style.removeProperty("height");
      style.removeProperty("block-size");
    }
    releaseAutoHeights(sourceChild, cloneChild);
  });
}

/**
 * The renderer waits for every image in the element to load. Lazy images that
 * are hidden or far off-screen (e.g. file-reference icons) never start on their
 * own, which stalled the render until its 30 s media timeout.
 */
function startPendingLazyImages(element: HTMLElement) {
  for (const image of Array.from(element.querySelectorAll("img"))) {
    if (!image.complete && image.loading === "lazy") image.loading = "eager";
  }
}

function paddedLength(computed: CSSStyleDeclaration, property: string) {
  return `${(Number.parseFloat(computed.getPropertyValue(property)) || 0) + IMAGE_PADDING}px`;
}

/**
 * Renders the element as it appears on screen (computed styles, fonts, images)
 * into a PNG. Layout is measured synchronously; the padding is added around the
 * existing content box so text wraps exactly as in the transcript.
 */
export function renderChatMessageImage(element: HTMLElement): Promise<Blob> {
  const ownerWindow = element.ownerDocument.defaultView ?? window;
  const rect = element.getBoundingClientRect();
  const width = rect.width + IMAGE_PADDING * 2;
  const height = rect.height + IMAGE_PADDING * 2;
  const scale = chatMessageImageScale(width, height, ownerWindow.devicePixelRatio);
  if (scale === null) return Promise.reject(new ChatMessageImageTooLargeError());
  const computed = ownerWindow.getComputedStyle(element);
  startPendingLazyImages(element);
  return domToBlob(element, {
    width,
    height,
    scale,
    backgroundColor: surfaceColorBehind(element),
    style: {
      paddingTop: paddedLength(computed, "padding-top"),
      paddingRight: paddedLength(computed, "padding-right"),
      paddingBottom: paddedLength(computed, "padding-bottom"),
      paddingLeft: paddedLength(computed, "padding-left"),
      // The context-menu highlight and size clamps of the live layout do not belong in the image.
      outline: "none",
      borderRadius: "0",
      minWidth: "0",
      maxWidth: "none",
      minHeight: "0",
      maxHeight: "none",
    },
    onCloneNode: (clone) => {
      if (clone.nodeType === Node.ELEMENT_NODE) releaseAutoHeights(element, clone as Element);
    },
  });
}

/**
 * Starts the clipboard write before awaiting the render so the request keeps the
 * click's focus. Falls back to the native clipboard in the desktop app, which
 * also works while another window (such as Unity) owns keyboard focus.
 */
export async function writePngToClipboard(png: Promise<Blob>, ownerWindow: Window): Promise<void> {
  const clipboard = ownerWindow.navigator?.clipboard;
  const ClipboardItemCtor = (ownerWindow as Window & { ClipboardItem?: typeof ClipboardItem }).ClipboardItem;
  let clipboardError: unknown = new Error("Clipboard image writing is unavailable");
  if (typeof clipboard?.write === "function" && typeof ClipboardItemCtor === "function") {
    try {
      await clipboard.write([new ClipboardItemCtor({ "image/png": png })]);
      return;
    } catch (error) {
      clipboardError = error;
    }
  }
  // A failed render is the real cause even when the clipboard reported it first.
  const blob = await png;
  if (!hasTauriWindowRuntime()) throw clipboardError;
  await invoke("write_clipboard_png", new Uint8Array(await blob.arrayBuffer()));
}

export function copyChatMessageImage(element: HTMLElement): Promise<void> {
  const ownerWindow = element.ownerDocument.defaultView ?? window;
  return writePngToClipboard(renderChatMessageImage(element), ownerWindow);
}
