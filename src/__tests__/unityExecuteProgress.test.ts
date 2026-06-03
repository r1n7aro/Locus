import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  formatUnityExecuteProgressPercent,
  parseUnityExecuteProgressOutput,
  UNITY_EXECUTE_PROGRESS_TAG,
} from "../composables/unityExecuteProgress";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function progressLine(payload: Record<string, unknown>) {
  return `<${UNITY_EXECUTE_PROGRESS_TAG}>${JSON.stringify(payload)}</${UNITY_EXECUTE_PROGRESS_TAG}>`;
}

describe("unityExecuteProgress", () => {
  it("extracts the latest progress event and hides progress markers from output", () => {
    const preview = parseUnityExecuteProgressOutput([
      "before",
      progressLine({
        active: true,
        title: "unity_execute async test",
        info: "Calculating primes... 825,000/2,500,000",
        progress: 0.33,
        revision: 8,
      }),
      "after",
      progressLine({
        active: true,
        title: "unity_execute async test",
        info: "Calculating primes... 1,250,000/2,500,000",
        progress: 0.5,
        revision: 9,
      }),
    ].join("\n"));

    expect(preview.displayOutput).toBe("before\nafter");
    expect(preview.progress?.title).toBe("unity_execute async test");
    expect(preview.progress?.info).toContain("1,250,000");
    expect(preview.progress?.revision).toBe(9);
    expect(preview.progress ? formatUnityExecuteProgressPercent(preview.progress) : "").toBe("50%");
  });

  it("clamps progress values for stable UI width and labels", () => {
    const preview = parseUnityExecuteProgressOutput(progressLine({
      active: true,
      title: "Scan",
      info: "Done",
      progress: 1.4,
      revision: 1,
    }));

    expect(preview.progress?.progress).toBe(1);
    expect(preview.progress ? formatUnityExecuteProgressPercent(preview.progress) : "").toBe("100%");
  });

  it("wires unity_execute progress through the bridge and tool block override", () => {
    const asyncExecuteSource = read("locus_unity/Editor/ExecuteCodeAsync/LocusBridge.ExecuteCodeAsync.cs");
    const agentSource = read("src-tauri/src/agent/instance/mod.rs");
    const unityBridgeSource = read("src-tauri/src/unity_bridge/mod.rs");
    const transportSource = read("src-tauri/src/unity_bridge/transport.rs");
    expect(read("src/components/tool-block-overrides/toolBlockOverrides.ts")).toContain("unity_execute");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("parseUnityExecuteProgressOutput");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("props.toolCall.progress");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("import hljs from \"../../hljs\";");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("hljs.highlight(code, { language: \"csharp\" }).value");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("v-html=\"highlightCSharp(codeArg)\"");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("class=\"tool-call-pre ui-select-text hljs\"");
    expect(read("src/assets/hljs-theme.css")).toContain("--md-syntax-subst: var(--md-code-fg);");
    expect(read("src/assets/hljs-theme.css")).toContain(":root .hljs-subst");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("const liveProgressHasValue = computed(() => typeof liveProgress.value?.progress === \"number\")");
    expect(unityBridgeSource).toContain("unity_execute_code_with_progress");
    expect(unityBridgeSource).toContain("cancel_execute_code");
    expect(unityBridgeSource).toContain("UNITY_EXECUTE_CANCELLED");
    expect(unityBridgeSource).toContain("Waiting for Locus Unity operation lock");
    expect(unityBridgeSource).toContain("Preparing Unity type index");
    expect(unityBridgeSource).toContain("Sending execute_code to Unity");
    expect(unityBridgeSource).toContain("Unity execute did not leave the sending stage within");
    expect(unityBridgeSource).toContain("Retrying execute_code after Unity pipe reconnect");
    expect(unityBridgeSource).toContain("reconnect_unity_pipe_for_execute");
    expect(unityBridgeSource).toContain("Unity execute progress was unavailable");
    expect(unityBridgeSource).toContain("disconnect_with_reason(project_path");
    expect(transportSource).toContain("disconnect_with_reason");
    expect(transportSource).toContain("fail_all_pending(conn, reason)");
    expect(agentSource).toContain("execute_unity_execute");
    expect(agentSource).toContain("unity_execute_code_with_progress_cancellable");
    expect(agentSource).toContain("StreamEvent::ToolCallProgress");
    expect(read("locus_unity/Editor/LocusBridge.cs")).toContain("execute_code_progress");
    expect(read("locus_unity/Editor/LocusBridge.cs")).toContain("cancel_execute_code");
    expect(agentSource).toContain("Waiting for Unity execute slot");
    expect(asyncExecuteSource).toContain("Checking compiler cache");
    expect(asyncExecuteSource).toContain("HandleCancelExecuteCode");
    expect(asyncExecuteSource).toContain("execute_code cancellation requested");
    expect(read("locus_unity/Editor/LocusBridge.ExecuteCode.cs")).toContain("Waiting for Unity main thread");
    expect(read("locus_unity/Editor/LocusBridge.ExecuteCode.cs")).toContain("ThrowIfExecuteCodeCanceled");
    expect(read("locus_unity/Editor/LocusBridge.ExecuteCode.cs")).toContain("Adding core compiler references");
    expect(read("locus_unity/Editor/LocusBridge.ExecuteCode.cs")).toContain("Adding precompiled assemblies");
    expect(read("locus_unity/Editor/LocusBridge.ExecuteCode.cs")).toContain("Adding ScriptAssemblies");
    expect(asyncExecuteSource).toContain("Compiling snippet");
    expect(asyncExecuteSource).toContain("Executing snippet");
    expect(asyncExecuteSource).toContain("Compilation failed");
    expect(asyncExecuteSource).toContain("Execution failed");
    expect(asyncExecuteSource).not.toContain("DisplayCancelableProgressBar(");
    expect(read("tools/unity_execute.json")).toContain("reports progress to the Locus tool call panel");
  });

  it("requires unity_execute request_editor_status like unity_run_states", () => {
    const unityExecuteDefinition = JSON.parse(read("tools/unity_execute.json"));
    const unityRunStatesDefinition = JSON.parse(read("tools/unity_run_states.json"));

    expect(unityExecuteDefinition.parameters.properties.request_editor_status).toEqual(
      unityRunStatesDefinition.parameters.properties.request_editor_status,
    );
    expect(unityExecuteDefinition.parameters.required).toContain("request_editor_status");
    expect(unityExecuteDefinition.parameters.required).not.toContain("editor_status");
  });

  it("keeps unity_execute progress above the collapsible tool details", () => {
    const source = read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue");
    const headerIndex = source.indexOf("class=\"tool-call-header");
    const progressIndex = source.indexOf("class=\"tool-call-progress-line");
    const detailIndex = source.indexOf("class=\"tool-call-detail");

    expect(headerIndex).toBeGreaterThanOrEqual(0);
    expect(progressIndex).toBeGreaterThanOrEqual(0);
    expect(detailIndex).toBeGreaterThanOrEqual(0);
    expect(headerIndex).toBeLessThan(progressIndex);
    expect(progressIndex).toBeLessThan(detailIndex);
    expect(source).toContain("v-if=\"showProgressLine\"");
    expect(source).toContain("const inlineStatus = computed(() => {");
    expect(source).toContain("class=\"tool-call-inline-dots\"");
    expect(source).toContain("v-if=\"infoExpanded && hasInfoDetail\"");
    expect(source).toContain("const isFramed = computed(() => infoExpanded.value || showProgressLine.value)");
    expect(source).toContain("'is-framed': isFramed");
    expect(source).toContain("class=\"tool-call-inline-status\"");
    expect(source).not.toContain("class=\"tool-call-waiting\"");
    expect(source).toContain("const infoExpanded = ref(false)");
    expect(source).not.toContain("collapseTimer");
    expect(source).not.toContain("1400");
    expect(source).toContain("class=\"unity-tool-call-block unity-execute-tool-block\"");
    expect(source).not.toContain("class=\"tool-call-block unity-execute-tool-block\"");
    expect(source).toMatch(/\.unity-tool-call-block\s*\{[\s\S]*border:\s*1px solid transparent/);
    expect(source).toMatch(/\.unity-tool-call-block\.is-framed\s*\{[\s\S]*border:\s*1px solid color-mix\(in srgb, #8b7cf6 46%, var\(--border-color\)\)/);
    expect(source).toMatch(/\.tool-call-detail\s*\{[\s\S]*padding:\s*6px 2px 0 20px/);
    expect(source).toMatch(/\.tool-call-progress-line\s*\{[\s\S]*padding:\s*5px 2px 0 20px/);
    expect(source).toMatch(/\.tool-call-progress-line\s*\{[\s\S]*border-top:\s*1px solid color-mix\(in srgb, var\(--border-color\) 58%, transparent\)/);
    expect(source).toMatch(/\.unity-execute-progress\s*\{[\s\S]*background:\s*transparent/);
  });
});
