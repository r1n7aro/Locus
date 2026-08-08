import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity embedded window setting", () => {
  it("defaults to enabled and destroys active windows when disabled", () => {
    const config = read("src-tauri/src/config.rs");
    const command = read("src-tauri/src/commands/unity_embed.rs");
    const app = read("src-tauri/src/lib.rs");

    expect(config).toContain("fn default_unity_embed_enabled()");
    expect(config).toContain("Arc::new(AtomicBool::new(true))");
    expect(command).toContain("pub async fn set_unity_embed_enabled");
    expect(command).toContain("destroy_unity_embed_control_window_on_main");
    expect(command).toContain("if !embed_enabled");
    expect(app).toContain("commands::get_unity_embed_enabled");
    expect(app).toContain("commands::set_unity_embed_enabled");
  });

  it("stops Unity-side HWND discovery through the project marker", () => {
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const unityWindow = read("locus_unity/Editor/LocusEditorWindow.cs");

    expect(bridge).toContain("UnityEmbed.disabled");
    expect(bridge).toContain("sync_unity_embed_enabled_marker");
    expect(unityWindow).toContain("RefreshEmbedFeatureState(false)");
    expect(unityWindow).toContain("if (!_frontendWindowConfigured || !_embedFeatureEnabled)");
    expect(unityWindow.indexOf("RefreshEmbedFeatureState(false)")).toBeLessThan(
      unityWindow.indexOf("SendOpenOrUpdate(false)", unityWindow.indexOf("private void SyncOverlay")),
    );
  });

  it("exposes the setting in Unity Connection with the shared switch", () => {
    const service = read("src/services/unity.ts");
    const settings = read("src/components/settings/UnityConnectionSettings.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(service).toContain('ipcInvoke<boolean>("get_unity_embed_enabled")');
    expect(service).toContain('ipcInvoke<boolean>("set_unity_embed_enabled", { value })');
    expect(settings).toContain("getUnityEmbedEnabled");
    expect(settings).toContain("setUnityEmbedEnabled");
    expect(settings).toContain("<BaseSwitch");
    expect(zh).toContain('"settings.unityConnection.embedLabel"');
    expect(en).toContain('"settings.unityConnection.embedLabel"');
  });
});
