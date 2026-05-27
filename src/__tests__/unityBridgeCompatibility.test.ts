import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("unityBridgeCompatibility", () => {
  it("uses a Unity 2020-only pipe accept fallback", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.cs");

    expect(bridge).toContain("#if UNITY_2020");
    expect(bridge).toContain("private const PipeOptions ServerPipeOptions = PipeOptions.None;");
    expect(bridge).toContain("private const PipeOptions ServerPipeOptions = PipeOptions.Asynchronous;");
    expect(bridge).toContain("WaitForConnectionCompat(server, ct);");
    expect(bridge).toContain("await server.WaitForConnectionAsync(ct);");
    expect(bridge).toContain("server.WaitForConnection();");
    expect(bridge).toContain("ct.Register(delegate");
  });

  it("keeps the Unity bridge connection stable after recompilation", () => {
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const transport = read("src-tauri/src/unity_bridge/transport.rs");

    expect(bridge).toContain("wait_for_unity_bridge_ready_after_recompile");
    expect(bridge).toContain("refresh_unity_type_index_after_recompile");
    expect(bridge).toContain("Unity reconnected after domain reload");
    expect(bridge).not.toContain("Unity recompile completed");
    expect(transport).toContain(".filter(|value| !value.is_empty())");
  });

  it("keeps transient View assemblies out of the Unity type index", () => {
    const typeIndex = read("locus_unity/Editor/LocusBridge.TypeIndex.cs");
    const viewScripts = read("locus_unity/Editor/LocusBridge.ViewScripts.cs");
    const bridge = read("locus_unity/Editor/LocusBridge.cs");

    expect(typeIndex).toContain('assemblyName.StartsWith("__LocusView_"');
    expect(typeIndex).toContain("IsInactiveSkillPackageAssemblyName(assemblyName)");
    expect(viewScripts).toContain("PreviousAssemblyId");
    expect(viewScripts).toContain("FindActiveSkillPackageAssembly");
    expect(viewScripts).toContain('\\"previousAssemblyId\\"');
    expect(viewScripts).toContain("HandleInvokeSkillPackage");
    expect(bridge).toContain("preprocessorSymbols: SnippetPreprocessorSymbols");
    expect(bridge).toContain("AddUnityVersionPreprocessorSymbols");
  });

  it("drops the cached Unity pipe connection after a response timeout", () => {
    const transport = read("src-tauri/src/unity_bridge/transport.rs");

    expect(transport).toContain('let err = "Unity response timed out".to_string();');
    expect(transport).toContain("drop(pending);");
    expect(transport).toContain("remove_connection_if_same(&conn.pipe_name, &conn).await;");
    expect(transport).toContain("close_connection(&conn, err.clone()).await;");
  });
});
