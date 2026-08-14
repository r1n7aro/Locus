import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const coordinator = readFileSync(
  "src-tauri/src/unity_hotreload/coordinator.rs",
  "utf8",
);

describe("Unity hot reload edit feedback", () => {
  it("reports edit and write status only while the hot reload pipeline is active", () => {
    expect(coordinator).toContain(
      "hot_reload_enabled && compiler_enabled",
    );

    const formatterStart = coordinator.indexOf(
      "pub async fn format_pending_edit_status",
    );
    const formatterEnd = coordinator.indexOf(
      "pub async fn on_recompile_converged",
      formatterStart,
    );
    const formatter = coordinator.slice(formatterStart, formatterEnd);

    expect(formatter).toContain(
      "if !edit_tracking_enabled(super::is_enabled(), crate::csharp_compile::is_enabled())",
    );
    expect(formatter.indexOf("edit_tracking_enabled")).toBeLessThan(
      formatter.indexOf("normalize_project_file_path"),
    );
    expect(formatter).not.toContain("Hot reload: disabled");
    expect(formatter).not.toContain("sidecar compiler is disabled");
  });
});
