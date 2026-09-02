import { emptyComposerIntent, hasComposerIntent } from "./chatInputIntents";
import type { UserMessageDraft } from "./chatMessageDraft";

const MAX_SHARED_COMPOSER_DRAFTS = 64;
const sharedComposerDrafts = new Map<string, UserMessageDraft>();

function normalizedKey(key: string | null | undefined): string {
  return key?.trim() ?? "";
}

function cloneComposerDraft(draft: UserMessageDraft): UserMessageDraft {
  return {
    text: draft.text,
    images: draft.images.map((image) => ({ ...image })),
    assetRefs: draft.assetRefs.map((assetRef) => ({ ...assetRef })),
    localFiles: draft.localFiles.map((file) => ({ ...file })),
    consoleTexts: draft.consoleTexts.map((entry) => ({ ...entry })),
    intent: {
      mode: draft.intent.mode,
      skills: draft.intent.skills.map((skill) => ({ ...skill })),
    },
  };
}

function emptyComposerDraft(text = ""): UserMessageDraft {
  return {
    text,
    images: [],
    assetRefs: [],
    localFiles: [],
    consoleTexts: [],
    intent: emptyComposerIntent(),
  };
}

function composerDraftHasContent(draft: UserMessageDraft): boolean {
  return !!draft.text
    || draft.images.length > 0
    || draft.assetRefs.length > 0
    || draft.localFiles.length > 0
    || draft.consoleTexts.length > 0
    || hasComposerIntent(draft.intent);
}

function touchComposerDraft(key: string, draft: UserMessageDraft): void {
  sharedComposerDrafts.delete(key);
  sharedComposerDrafts.set(key, draft);
  while (sharedComposerDrafts.size > MAX_SHARED_COMPOSER_DRAFTS) {
    const oldest = sharedComposerDrafts.keys().next().value as string | undefined;
    if (!oldest) break;
    sharedComposerDrafts.delete(oldest);
  }
}

export function readSharedComposerDraft(key: string | null | undefined): UserMessageDraft | null {
  const normalized = normalizedKey(key);
  if (!normalized) return null;
  const draft = sharedComposerDrafts.get(normalized);
  if (!draft) return null;
  touchComposerDraft(normalized, draft);
  return cloneComposerDraft(draft);
}

export function writeSharedComposerDraft(
  key: string | null | undefined,
  draft: UserMessageDraft,
): void {
  const normalized = normalizedKey(key);
  if (!normalized) return;
  if (!composerDraftHasContent(draft)) {
    sharedComposerDrafts.delete(normalized);
    return;
  }
  touchComposerDraft(normalized, cloneComposerDraft(draft));
}

export function updateSharedComposerDraftText(
  key: string | null | undefined,
  text: string,
): void {
  const normalized = normalizedKey(key);
  if (!normalized) return;
  const existing = sharedComposerDrafts.get(normalized);
  if (!existing) {
    if (text) touchComposerDraft(normalized, emptyComposerDraft(text));
    return;
  }
  const next = { ...existing, text };
  if (!composerDraftHasContent(next)) {
    sharedComposerDrafts.delete(normalized);
    return;
  }
  touchComposerDraft(normalized, next);
}

export function clearSharedComposerDraft(key: string | null | undefined): void {
  const normalized = normalizedKey(key);
  if (normalized) sharedComposerDrafts.delete(normalized);
}
