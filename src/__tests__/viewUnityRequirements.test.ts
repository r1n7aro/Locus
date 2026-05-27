import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View Unity requirements", () => {
  it("declares and enforces Unity connection requirements before View runtime render", () => {
    const service = read("src/services/view.ts");
    const host = read("src/components/ViewHostWindow.vue");
    const page = read("src/components/ViewPackageView.vue");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const runtime = read("src-tauri/src/view.rs");
    const templates = read("src-tauri/src/view/templates/mod.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(service).toContain("export interface ViewRequirements");
    expect(service).toContain("unityConnection: boolean;");
    expect(service).toContain("export function viewRequiresUnityConnection");
    expect(service).toContain("export async function checkViewOpenRequirements");
    expect(service).toContain("export function normalizeViewError");
    expect(service).toContain("view.error.unityConnectionRequiredNamed");

    expect(templates).toContain("requirements: Some(ViewRequirements");
    expect(templates).toContain("\"inspector-form\" | \"field-blocks\" | \"node-graph\" | \"serialized-table\"");

    expect(runtime).toContain("pub requirements: Option<ViewRequirements>");
    expect(runtime).toContain("ensure_view_open_requirements");
    expect(runtime).toContain("query_unity_connection_status");
    expect(runtime).toContain("requires a Unity Editor connection");

    expect(host).toContain("checkUnityConnectionStatus");
    expect(host).toContain("viewRequiresUnityConnection(next.manifest)");
    expect(host).toContain("view.host.unityConnectionRequired");

    expect(page).toContain("selectedViewUnityRequirementText");
    expect(page).toContain("checkViewOpenRequirements(view)");
    expect(page).toContain("normalizeViewError(runError");
    expect(page).toContain("view.metadata.unityConnection");
    expect(sessionPanel).toContain("checkViewOpenRequirements(view)");
    expect(sessionPanel).toContain("normalizeViewError(error");
    expect(sessionPanel).toContain("cachedViewOpenRequirementError(view)");
    expect(sessionPanel).toContain("projectStore.unityConnectionStatus");
    expect(sessionPanel).toContain("viewUnityConnectionRequiredError(view.name)");
    expect(sessionPanel).not.toContain("v-else-if=\"viewError\"");
    expect(sessionPanel).not.toContain("class=\"sp-view-empty is-error\"");

    expect(zh).toContain('"view.metadata.unityConnection": "Unity 连接"');
    expect(zh).toContain('"view.error.unityConnectionRequiredNamed": "视图“{0}”需要 Unity 编辑器连接。连接 Unity 后再打开。"');
    expect(en).toContain('"view.metadata.unityConnection": "Unity Connection"');
    expect(en).toContain('"view.error.unityConnectionRequiredNamed": "View \\"{0}\\" requires a Unity Editor connection. Connect Unity, then open it again."');
  });
});
