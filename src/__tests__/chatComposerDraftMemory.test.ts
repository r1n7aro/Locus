import { afterEach, describe, expect, it } from "vitest";
import {
  clearSharedComposerDraft,
  readSharedComposerDraft,
  updateSharedComposerDraftText,
  writeSharedComposerDraft,
} from "../composables/chatComposerDraftMemory";
import type { UserMessageDraft } from "../composables/chatMessageDraft";

const TEST_KEY = "workbench:test-editor";

function attachmentDraft(): UserMessageDraft {
  return {
    text: "inspect this",
    images: [{ data: "cG5n", mimeType: "image/png" }],
    assetRefs: [{ path: "Assets/Player.prefab", kind: "asset", source: "manual" }],
    localFiles: [{ path: "C:/temp/report.txt", isDir: false, name: "report.txt" }],
    consoleTexts: [{ title: "Unity Console", source: "unity", level: "Error", text: "boom" }],
    intent: {
      mode: "plan",
      skills: [{ dirName: "review", name: "Review", source: "project" }],
    },
  };
}

describe("shared composer draft memory", () => {
  afterEach(() => clearSharedComposerDraft(TEST_KEY));

  it("restores a complete attachment draft without sharing mutable objects", () => {
    const draft = attachmentDraft();
    writeSharedComposerDraft(TEST_KEY, draft);

    draft.localFiles[0]!.path = "C:/changed.txt";
    const restored = readSharedComposerDraft(TEST_KEY);
    expect(restored).toEqual(attachmentDraft());

    restored!.images[0]!.data = "changed";
    expect(readSharedComposerDraft(TEST_KEY)?.images[0]?.data).toBe("cG5n");
  });

  it("updates shared text while retaining attachments and clears an empty draft", () => {
    writeSharedComposerDraft(TEST_KEY, attachmentDraft());
    updateSharedComposerDraftText(TEST_KEY, "updated");

    expect(readSharedComposerDraft(TEST_KEY)).toMatchObject({
      text: "updated",
      localFiles: [{ path: "C:/temp/report.txt" }],
    });

    writeSharedComposerDraft(TEST_KEY, {
      text: "",
      images: [],
      assetRefs: [],
      localFiles: [],
      consoleTexts: [],
      intent: { mode: "build", skills: [] },
    });
    expect(readSharedComposerDraft(TEST_KEY)).toBeNull();
  });
});
