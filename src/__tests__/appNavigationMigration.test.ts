import { describe, expect, it } from "vitest";
import { migrateAppPage } from "../stores/ui";

describe("app navigation migration", () => {
  it.each(["project", "chat", "collab", "knowledge", "asset", "views"])(
    "migrates legacy workspace page %s to development",
    (legacyPage) => {
      expect(migrateAppPage(legacyPage)).toBe("development");
    },
  );

  it.each(["development", "plugins", "agent", "settings"] as const)(
    "keeps process page %s",
    (page) => {
      expect(migrateAppPage(page)).toBe(page);
    },
  );
});
