import { describe, expect, it } from "vitest";
import {
  sessionContextExportFileName,
  sessionContextExportTitleFragment,
} from "../composables/sessionContextExport";

describe("session context export file names", () => {
  it("includes the short session id and readable title", () => {
    expect(sessionContextExportFileName("6201ad9e-1234", "修复角色移动"))
      .toBe("context_6201ad9e_修复角色移动.yaml");
  });

  it("filters Windows-invalid characters and trailing punctuation", () => {
    expect(sessionContextExportFileName("abcdef12-1234", '场景: Player/A*?  .'))
      .toBe("context_abcdef12_场景_Player_A.yaml");
  });

  it("keeps the title fragment bounded and provides an empty-title fallback", () => {
    expect(Array.from(sessionContextExportTitleFragment("很长".repeat(80)))).toHaveLength(72);
    expect(sessionContextExportFileName("abcdef12", "..."))
      .toBe("context_abcdef12_untitled.yaml");
  });
});
