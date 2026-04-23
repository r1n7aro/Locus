import { describe, expect, it } from "vitest";
import { normalizeLocale, resolveLocale } from "../i18n";

describe("i18n locale resolution", () => {
  it("normalizes supported locale tags", () => {
    expect(normalizeLocale("zh-CN")).toBe("zh");
    expect(normalizeLocale("zh_Hans")).toBe("zh");
    expect(normalizeLocale("en-US")).toBe("en");
  });

  it("returns null for unsupported locale tags", () => {
    expect(normalizeLocale("ja-JP")).toBeNull();
    expect(normalizeLocale("")).toBeNull();
    expect(normalizeLocale(null)).toBeNull();
  });

  it("prefers saved locale over system locale", () => {
    expect(resolveLocale({
      savedLocale: "zh",
      systemLocale: "en-US",
      navigatorLocales: ["en-US"],
    })).toBe("zh");
  });

  it("uses system locale when no saved locale exists", () => {
    expect(resolveLocale({
      systemLocale: "zh-CN",
      navigatorLocales: ["en-US"],
    })).toBe("zh");
  });

  it("falls back to navigator locale before english", () => {
    expect(resolveLocale({
      systemLocale: "ja-JP",
      navigatorLocales: ["en-US"],
    })).toBe("en");
  });

  it("falls back to english when no locale source is supported", () => {
    expect(resolveLocale({
      systemLocale: "ja-JP",
      navigatorLocales: ["fr-FR"],
    })).toBe("en");
  });
});
