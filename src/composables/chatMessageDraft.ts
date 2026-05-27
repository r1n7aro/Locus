import type {
  AssetRefAttachment,
  ChatMessage,
  ImageAttachment,
  SkillIntentItem,
} from "../types";
import { extractChatAssetRefs } from "./chatAssetRefs";
import {
  displayUserMessageContent,
  userMessageConsoleEntries,
  userMessageLocalFileEntries,
} from "./chatUserMessageDisplay";
import {
  dedupeSkillIntents,
  emptyComposerIntent,
  parseUserIntentMeta,
  type ComposerIntentState,
} from "./chatInputIntents";

export const LOCUS_CHAT_MESSAGE_DRAFT_MIME = "application/x-locus-chat-message-draft";

const CLIPBOARD_HTML_MARKER_RE =
  /<!--\s*locus-user-message-draft:([A-Za-z0-9+/=]+)\s*-->/;
const SERIALIZED_KIND = "locus_user_message_draft_v1";
const UNITY_ASSET_REF_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;
const UNITY_SCENE_OBJECT_REF_RE = /^((?:Assets|Packages)\/.+?\.unity)\/.+/i;
const PROJECT_KNOWLEDGE_REF_ROOT_RE = /^(?:design|memory|skill|reference)\/.+\.md$/i;

export interface UserMessageDraftLocalFile {
  path: string;
  isDir: boolean;
  name?: string;
  typeLabel?: string;
  source?: string;
}

export interface UserMessageDraftConsoleText {
  title: string;
  source: string;
  level: string;
  text: string;
}

export interface UserMessageDraft {
  text: string;
  images: ImageAttachment[];
  assetRefs: AssetRefAttachment[];
  localFiles: UserMessageDraftLocalFile[];
  consoleTexts: UserMessageDraftConsoleText[];
  intent: ComposerIntentState;
}

interface SerializedUserMessageDraft extends UserMessageDraft {
  kind: typeof SERIALIZED_KIND;
}

export interface ChatMessageClipboardPayload {
  text: string;
  draft: UserMessageDraft | null;
  serializedDraft: string | null;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function normalizePath(path: string) {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function pathBaseName(path: string) {
  return path.split("/").filter(Boolean).pop();
}

function normalizeAssetRefSource(source: unknown): AssetRefAttachment["source"] {
  return source === "manual" ? "manual" : source === "unity" ? "unity" : undefined;
}

function inferAssetRefKind(path: string, kind: unknown): AssetRefAttachment["kind"] {
  if (kind === "knowledge") return "knowledge";
  if (kind === "sceneObject") return "sceneObject";
  if (PROJECT_KNOWLEDGE_REF_ROOT_RE.test(path)) return "knowledge";
  return UNITY_SCENE_OBJECT_REF_RE.test(path) ? "sceneObject" : "asset";
}

function normalizeAssetRef(value: unknown): AssetRefAttachment | null {
  if (!isObject(value) || typeof value.path !== "string") return null;
  const path = normalizePath(value.path);
  if (!path) return null;
  return {
    path,
    kind: inferAssetRefKind(path, value.kind),
    name: typeof value.name === "string" && value.name.trim() ? value.name.trim() : undefined,
    typeLabel: typeof value.typeLabel === "string" && value.typeLabel.trim()
      ? value.typeLabel.trim()
      : undefined,
    source: normalizeAssetRefSource(value.source) ?? "manual",
  };
}

function assetRefFromInlinePath(pathValue: string): AssetRefAttachment | null {
  const path = normalizePath(pathValue);
  if (!path) return null;
  if (PROJECT_KNOWLEDGE_REF_ROOT_RE.test(path)) {
    return {
      path,
      kind: "knowledge",
      name: pathBaseName(path),
      source: "manual",
    };
  }
  if (UNITY_ASSET_REF_ROOT_RE.test(path)) {
    return {
      path,
      kind: UNITY_SCENE_OBJECT_REF_RE.test(path) ? "sceneObject" : "asset",
      name: pathBaseName(path),
      source: "manual",
    };
  }
  return null;
}

function dedupeAssetRefs(assetRefs: AssetRefAttachment[]) {
  const seen = new Set<string>();
  const deduped: AssetRefAttachment[] = [];
  for (const assetRef of assetRefs) {
    const normalized = normalizeAssetRef(assetRef);
    if (!normalized) continue;
    const key = `${normalized.kind}:${normalized.path.toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.add(key);
    deduped.push(normalized);
  }
  return deduped;
}

function userMessageAssetRefs(message: ChatMessage) {
  const assetRefs = [...(message.assetRefs ?? [])];
  const inlineRefs = extractChatAssetRefs(message.content).refs;
  for (const path of inlineRefs) {
    const assetRef = assetRefFromInlinePath(path);
    if (assetRef) assetRefs.push(assetRef);
  }
  return dedupeAssetRefs(assetRefs);
}

function normalizeImage(value: unknown): ImageAttachment | null {
  if (!isObject(value)) return null;
  if (typeof value.data !== "string" || typeof value.mimeType !== "string") return null;
  if (!value.data || !value.mimeType) return null;
  return {
    data: value.data,
    mimeType: value.mimeType,
  };
}

function normalizeLocalFile(value: unknown): UserMessageDraftLocalFile | null {
  if (!isObject(value) || typeof value.path !== "string") return null;
  const path = normalizePath(value.path);
  if (!path) return null;
  return {
    path,
    isDir: !!value.isDir,
    name: typeof value.name === "string" && value.name.trim() ? value.name.trim() : undefined,
    typeLabel: typeof value.typeLabel === "string" && value.typeLabel.trim()
      ? value.typeLabel.trim()
      : undefined,
    source: typeof value.source === "string" && value.source.trim() ? value.source.trim() : undefined,
  };
}

function normalizeConsoleText(value: unknown): UserMessageDraftConsoleText | null {
  if (!isObject(value)) return null;
  const text = typeof value.text === "string" ? value.text.trim() : "";
  if (!text) return null;
  return {
    title: typeof value.title === "string" && value.title.trim()
      ? value.title.trim()
      : "Unity Console",
    source: typeof value.source === "string" && value.source.trim()
      ? value.source.trim()
      : "unity-console",
    level: typeof value.level === "string" && value.level.trim() ? value.level.trim() : "Log",
    text,
  };
}

function normalizeSkillIntent(value: unknown): SkillIntentItem | null {
  if (!isObject(value)) return null;
  const source = value.source === "project" ? "project" : value.source === "app" ? "app" : null;
  if (!source || typeof value.dirName !== "string" || typeof value.name !== "string") return null;
  const dirName = value.dirName.trim();
  const name = value.name.trim();
  if (!dirName || !name) return null;
  return { source, dirName, name };
}

function normalizeIntent(value: unknown): ComposerIntentState {
  if (!isObject(value)) return emptyComposerIntent();
  const mode = value.mode === "plan" ? "plan" : "build";
  const skills = Array.isArray(value.skills)
    ? value.skills
      .map(normalizeSkillIntent)
      .filter((skill): skill is SkillIntentItem => !!skill)
    : [];
  return {
    mode,
    skills: dedupeSkillIntents(skills),
  };
}

function userMessageIntent(message: ChatMessage): ComposerIntentState {
  const meta = message.intentMeta ?? parseUserIntentMeta(message.thinkingSignature);
  if (!meta) return emptyComposerIntent();
  return {
    mode: meta.mode,
    skills: dedupeSkillIntents(meta.skills),
  };
}

function normalizeUserMessageDraft(value: unknown): UserMessageDraft | null {
  if (!isObject(value)) return null;
  const text = typeof value.text === "string" ? value.text : "";
  const images = Array.isArray(value.images)
    ? value.images.map(normalizeImage).filter((image): image is ImageAttachment => !!image)
    : [];
  const assetRefs = Array.isArray(value.assetRefs)
    ? dedupeAssetRefs(value.assetRefs)
    : [];
  const localFiles = Array.isArray(value.localFiles)
    ? value.localFiles
      .map(normalizeLocalFile)
      .filter((file): file is UserMessageDraftLocalFile => !!file)
    : [];
  const consoleTexts = Array.isArray(value.consoleTexts)
    ? value.consoleTexts
      .map(normalizeConsoleText)
      .filter((entry): entry is UserMessageDraftConsoleText => !!entry)
    : [];

  return {
    text,
    images,
    assetRefs,
    localFiles,
    consoleTexts,
    intent: normalizeIntent(value.intent),
  };
}

function serializeUserMessageDraft(draft: UserMessageDraft) {
  const normalized = normalizeUserMessageDraft(draft);
  if (!normalized) return null;
  const payload: SerializedUserMessageDraft = {
    kind: SERIALIZED_KIND,
    ...normalized,
  };
  return JSON.stringify(payload);
}

function parseSerializedUserMessageDraft(raw: string | null | undefined) {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!isObject(parsed) || parsed.kind !== SERIALIZED_KIND) return null;
    return normalizeUserMessageDraft(parsed);
  } catch {
    return null;
  }
}

function encodeBase64Utf8(value: string) {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

function decodeBase64Utf8(value: string) {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return new TextDecoder().decode(bytes);
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function buildClipboardHtml(text: string, serializedDraft: string | null) {
  const marker = serializedDraft
    ? `<!--locus-user-message-draft:${encodeBase64Utf8(serializedDraft)}-->`
    : "";
  const htmlText = escapeHtml(text).replace(/\r?\n/g, "<br>");
  return `${marker}<span>${htmlText}</span>`;
}

function parseClipboardHtmlDraft(html: string | null | undefined) {
  const marker = html?.match(CLIPBOARD_HTML_MARKER_RE)?.[1];
  if (!marker) return null;
  try {
    return parseSerializedUserMessageDraft(decodeBase64Utf8(marker));
  } catch {
    return null;
  }
}

function withClipboardCopyEvent(text: string, serializedDraft: string | null) {
  if (typeof document === "undefined") return false;
  const body = document.body;
  if (!body) return false;

  const activeElement = document.activeElement instanceof HTMLElement
    ? document.activeElement
    : null;
  const selection = document.getSelection();
  const ranges: Range[] = [];
  if (selection) {
    for (let index = 0; index < selection.rangeCount; index += 1) {
      ranges.push(selection.getRangeAt(index).cloneRange());
    }
  }

  let copied = false;
  const textarea = document.createElement("textarea");
  textarea.value = text || " ";
  textarea.setAttribute("readonly", "true");
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  textarea.style.top = "0";

  const onCopy = (event: ClipboardEvent) => {
    if (!event.clipboardData) return;
    event.clipboardData.setData("text/plain", text);
    event.clipboardData.setData("text/html", buildClipboardHtml(text, serializedDraft));
    if (serializedDraft) {
      event.clipboardData.setData(LOCUS_CHAT_MESSAGE_DRAFT_MIME, serializedDraft);
    }
    event.preventDefault();
    copied = true;
  };

  document.addEventListener("copy", onCopy);
  body.appendChild(textarea);
  textarea.focus();
  textarea.select();

  try {
    return document.execCommand("copy") && copied;
  } finally {
    document.removeEventListener("copy", onCopy);
    textarea.remove();
    if (selection) {
      selection.removeAllRanges();
      for (const range of ranges) {
        selection.addRange(range);
      }
    }
    activeElement?.focus({ preventScroll: true });
  }
}

async function writeClipboardItem(text: string, serializedDraft: string | null) {
  if (
    typeof navigator === "undefined"
    || !navigator.clipboard?.write
    || typeof ClipboardItem === "undefined"
  ) {
    return false;
  }
  const html = buildClipboardHtml(text, serializedDraft);
  await navigator.clipboard.write([
    new ClipboardItem({
      "text/plain": new Blob([text], { type: "text/plain" }),
      "text/html": new Blob([html], { type: "text/html" }),
    }),
  ]);
  return true;
}

export function buildUserMessageDraft(message: ChatMessage): UserMessageDraft {
  return {
    text: displayUserMessageContent(message.content),
    images: (message.images ?? []).map((image) => ({
      data: image.data,
      mimeType: image.mimeType,
    })),
    assetRefs: userMessageAssetRefs(message),
    localFiles: userMessageLocalFileEntries(message.content).map((file) => ({
      path: file.path,
      isDir: file.kind === "folder",
      typeLabel: file.typeLabel || undefined,
      source: "message",
    })),
    consoleTexts: userMessageConsoleEntries(message.content).map((entry) => ({
      title: entry.title,
      source: entry.source,
      level: entry.level,
      text: entry.text,
    })),
    intent: userMessageIntent(message),
  };
}

export function copyableChatMessageText(message: ChatMessage) {
  return message.role === "user"
    ? displayUserMessageContent(message.content)
    : message.content;
}

export function buildChatMessageClipboardPayload(message: ChatMessage): ChatMessageClipboardPayload {
  const text = copyableChatMessageText(message);
  const draft = message.role === "user" ? buildUserMessageDraft(message) : null;
  return {
    text,
    draft,
    serializedDraft: draft ? serializeUserMessageDraft(draft) : null,
  };
}

export function readUserMessageDraftFromClipboardData(data: DataTransfer | null | undefined) {
  if (!data) return null;
  return parseSerializedUserMessageDraft(data.getData(LOCUS_CHAT_MESSAGE_DRAFT_MIME))
    ?? parseSerializedUserMessageDraft(data.getData(LOCUS_CHAT_MESSAGE_DRAFT_MIME.toLowerCase()))
    ?? parseClipboardHtmlDraft(data.getData("text/html"));
}

export async function writeChatMessageClipboard(payload: ChatMessageClipboardPayload) {
  try {
    if (withClipboardCopyEvent(payload.text, payload.serializedDraft)) {
      return;
    }
  } catch {
    // Fall back to async clipboard APIs below.
  }
  try {
    if (await writeClipboardItem(payload.text, payload.serializedDraft)) {
      return;
    }
  } catch {
    // Fall back to plain text below.
  }
  await navigator.clipboard.writeText(payload.text);
}
