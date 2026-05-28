import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat composer draft persistence", () => {
  it("stores drafts per session and restores the new-chat draft when switching back", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain('const NEW_CHAT_DRAFT_KEY = "__new_chat__";');
    expect(chatView).toContain("const composerDrafts = ref(new Map<string, string>());");
    expect(chatView).toContain("function draftSessionKey(sessionId: string | null)");
    expect(chatView).toContain("watch(inputText, (value) => {");
    expect(chatView).toContain("storeComposerDraft(props.activeSessionId, value);");
    expect(chatView).toContain("void restoreComposerDraft(nextSessionId ?? null);");
    expect(chatView).toContain("if (props.activeSessionId === null) {");
  });

  it("syncs asset reference drafts between chat windows by session", () => {
    const chatView = read("src/components/ChatView.vue");
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(chatView).toContain("const composerAssetRefSyncKey = computed(() => `chat:${draftSessionKey(props.activeSessionId)}`);");
    expect(chatView).toContain(':asset-ref-sync-key="composerAssetRefSyncKey"');
    expect(richInput).toContain('const ASSET_REF_SYNC_CHANNEL = "locus-chat-asset-ref-drafts";');
    expect(richInput).toContain("function setAssetRefAttachments(");
    expect(richInput).toContain("broadcastAssetRefDraft(next);");
    expect(richInput).toContain("function removeAssetRef(index: number)");
    expect(richInput).toContain("setAssetRefAttachments(next);");
    expect(richInput).toContain("applyAssetRefSyncMessage(event.data);");
    expect(richInput).toContain("const RECENT_ASSET_REF_REMOVAL_SUPPRESS_MS = 100;");
    expect(richInput).toContain("recentlyRemovedAssetRefKeys");
    expect(richInput).toContain("respectRecentRemoval: true");
  });

  it("converts Unity mention refs into composer asset references", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(richInput).toContain('import { buildProjectKnowledgeRefPath, extractChatAssetRefs } from "../../composables/chatAssetRefs";');
    expect(richInput).toContain("const UNITY_ASSET_REF_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\\/|$)/i;");
    expect(richInput).toContain("const assetRef = buildManualAssetRef(mentionPath);");
    expect(richInput).toContain("addAssetRefs([assetRef]);");
    expect(richInput).toContain("const inlineAssetRefs = extractInlineUnityAssetRefs(parsed.cleanedText);");
    expect(richInput).toContain("const cleanedInput = normalizeComposerText(inlineAssetRefs.text);");
    expect(richInput).toContain("dedupeAssetRefs([...assetRefAttachments.value, ...inlineAssetRefs.assetRefs]);");
  });

  it("converts knowledge search results into composer asset references", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(richInput).toContain("const refPath = buildProjectKnowledgeRefPath(result.type, result.path);");
    expect(richInput).toContain("relPath: refPath,");
    expect(richInput).toContain("parentPath: parentPathFor(refPath),");
    expect(richInput).toContain("meta: refPath,");
  });
});
