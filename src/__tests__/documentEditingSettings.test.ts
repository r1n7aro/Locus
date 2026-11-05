// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { installFakeSettingsBackend } from "./fixtures/settingsBackend";

const key = "locus-document-editing-settings";
beforeEach(() => { localStorage.clear(); vi.resetModules(); });

describe("document editing settings", () => {
  it("defaults to disabled for absent and non-boolean preferences", async () => {
    const { normalizeDocumentEditingSettings, useDocumentEditingSettings } = await import("../composables/useDocumentEditingSettings");
    expect(useDocumentEditingSettings().state.punctuationPairing).toBe(false);
    for (const value of [null, {}, { punctuationPairing: "true" }, { punctuationPairing: 1 }]) {
      expect(normalizeDocumentEditingSettings(value).punctuationPairing).toBe(false);
    }
  });

  it("defaults to disabled when saved JSON is malformed", async () => {
    localStorage.setItem(key, "{invalid");
    expect((await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings().state.punctuationPairing).toBe(false);
  });

  it("persists both enabled and disabled choices across reloads", async () => {
    const first = (await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings();
    first.setPunctuationPairing(true);
    vi.resetModules();
    const second = (await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings();
    expect(second.state.punctuationPairing).toBe(true);
    second.setPunctuationPairing(false);
    vi.resetModules();
    expect((await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings().state.punctuationPairing).toBe(false);
  });

  it("persists a normalized comment author name", async () => {
    const first = (await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings();
    expect(first.state.commentAuthor).toBe("");
    first.setCommentAuthor("  Enno \n Ruschak  ");
    expect(first.state.commentAuthor).toBe("Enno Ruschak");
    vi.resetModules();
    const second = (await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings();
    expect(second.state.commentAuthor).toBe("Enno Ruschak");
    expect(second.state.punctuationPairing).toBe(false);
  });

  it("follows changes from other windows and a settings reset", async () => {
    const backend = await installFakeSettingsBackend({ "documents.punctuationPairing": false });
    const settings = (await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings();
    await backend.settle();
    backend.change("documents.punctuationPairing", true);
    expect(settings.state.punctuationPairing).toBe(true);
    backend.change("documents.punctuationPairing", false);
    expect(settings.state.punctuationPairing).toBe(false);
    backend.uninstall();
  });

  it("writes through the settings backend and imports the old preference once", async () => {
    localStorage.setItem(key, JSON.stringify({ punctuationPairing: true, commentAuthor: "Enno" }));
    const backend = await installFakeSettingsBackend();
    const settings = (await import("../composables/useDocumentEditingSettings")).useDocumentEditingSettings();
    await backend.settle();
    expect(backend.imports).toEqual([{ "documents.punctuationPairing": true, "documents.commentAuthor": "Enno" }]);
    expect(settings.state).toEqual({ punctuationPairing: true, commentAuthor: "Enno" });
    settings.setCommentAuthor("  Other  Name ");
    await backend.settle();
    expect(backend.values["documents.commentAuthor"]).toBe("Other Name");
    backend.uninstall();
  });
});
