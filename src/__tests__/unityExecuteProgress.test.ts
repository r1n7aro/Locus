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
    expect(read("src/components/tool-block-overrides/toolBlockOverrides.ts")).toContain("unity_execute");
    expect(read("src/components/tool-block-overrides/UnityExecuteToolBlock.vue")).toContain("parseUnityExecuteProgressOutput");
    expect(read("src-tauri/src/unity_bridge/mod.rs")).toContain("unity_execute_code_with_progress");
    expect(read("src-tauri/src/agent/instance/mod.rs")).toContain("execute_unity_execute");
    expect(read("locus_unity/Editor/LocusBridge.cs")).toContain("execute_code_progress");
    expect(read("locus_unity/Editor/ExecuteCodeAsync/LocusBridge.ExecuteCodeAsync.cs")).not.toContain("DisplayCancelableProgressBar(");
    expect(read("tools/unity_execute.json")).toContain("reports progress to the Locus tool call panel");
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
