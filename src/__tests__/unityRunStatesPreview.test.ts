import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  buildUnityRunStatesRuntimePreview,
  parseUnityRunStatesArguments,
  parseUnityRunStatesOutput,
} from "../composables/unityRunStatesPreview";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("unityRunStatesPreview", () => {
  it("formats states into stable start update end phases", () => {
    const preview = parseUnityRunStatesArguments(JSON.stringify({
      request_editor_status: "playing",
      initial_state: "wait_player",
      states: [
        {
          name: "wait_player",
          variables: "int checks = 0;",
          start: "ctx.PromptUser(\"jump_debug\", \"press jump\");",
          update: "checks += 1;\nif (ready) { ctx.Goto(\"jump_once\"); return; }\nctx.Sleep(1);",
          end: "",
        },
        {
          name: "jump_once",
          variables: "int visits = 0;",
          update: "visits += 1; var hits = ctx.Global(\"hits\", 0); hits.Value += 1; ctx.Print($\"frame={ctx.TotalFrames},hits={hits.Value},visits={visits}\"); ctx.Done(\"ok\");",
        },
      ],
    }));

    expect(preview?.requestEditorStatus).toBe("playing");
    expect(preview?.initialState).toBe("wait_player");
    expect(preview?.states[0]?.isInitial).toBe(true);
    expect(preview?.states[0]?.phases.map((phase) => phase.key)).toEqual([
      "variables",
      "start",
      "update",
      "end",
    ]);
    expect(preview?.states[0]?.phases[0]?.code).toContain("int checks = 0;");
    expect(preview?.states[0]?.phases[2]?.code).toContain("if (ready) {");
    expect(preview?.states[0]?.phases[2]?.code).toContain("  ctx.Goto(\"jump_once\");");
    expect(preview?.states[0]?.phases[3]?.empty).toBe(true);
    expect(preview?.states[1]?.phases[0]?.code).toContain("int visits = 0;");
    expect(preview?.states[1]?.phases[2]?.code).toContain("ctx.Global(\"hits\", 0)");
  });

  it("parses run output into summary fields and prints", () => {
    const preview = parseUnityRunStatesOutput([
      "status: ok",
      "final_state: jump_once",
      "frames: 93",
      "duration_ms: 476",
      "message: done",
      "prints:",
      "frame,t,state,posY",
      "21,0.179,JumpAscending,0.213",
    ].join("\n"));

    expect(preview?.fields.map((field) => field.key)).toEqual([
      "status",
      "final_state",
      "frames",
      "duration_ms",
      "message",
    ]);
    expect(preview?.fields[1]?.label).toBe("final state");
    expect(preview?.prints).toContain("frame,t,state,posY");
  });

  it("does not present argument snippets as live runtime progress before output arrives", () => {
    const args = JSON.stringify({
      request_editor_status: "playing",
      initial_state: "wait_player",
      states: [
        {
          name: "wait_player",
          start: [
            "ctx.PromptUser(\"jump_debug\", \"press jump\");",
            "ctx.Print(\"ready for jump\");",
          ].join("\n"),
          update: "ctx.Goto(\"jump_once\");",
        },
        {
          name: "jump_once",
          update: "ctx.Print($\"frame={ctx.TotalFrames}\"); ctx.Done(\"ok\");",
        },
      ],
    });

    const running = buildUnityRunStatesRuntimePreview(args, undefined, "running");
    expect(running).toBeNull();

    const done = buildUnityRunStatesRuntimePreview(args, [
      "status: ok",
      "final_state: jump_once",
      "message: ok",
      "prints:",
      "frame=12",
    ].join("\n"), "done");
    expect(done?.currentState).toBe("jump_once");
    expect(done?.finalStatus).toBe("ok");
    expect(done?.finalMessage).toBe("ok");
    expect(done?.printText).toBe("frame=12");
    expect(done?.printCount).toBe(1);
    expect(done?.isFinal).toBe(true);
  });

  it("shows large print output metadata instead of static print hints", () => {
    const args = JSON.stringify({
      request_editor_status: "playing",
      initial_state: "sample",
      states: [
        {
          name: "sample",
          update: "ctx.Print(\"fallback\"); ctx.Done(\"ok\");",
        },
      ],
    });

    const preview = buildUnityRunStatesRuntimePreview(args, [
      "status: ok",
      "final_state: sample",
      "print_lines: 12000",
      "print_tokens_estimate: 100001",
      "print_output: too large",
      "result_file: F:\\Project\\Library\\Locus\\RunStates\\run-states.txt",
    ].join("\n"), "done");

    expect(preview?.printText).toContain("too large");
    expect(preview?.printText).toContain("12000 lines");
    expect(preview?.printText).toContain("run-states.txt");
    expect(preview?.printText).not.toContain("fallback");
    expect(preview?.printCount).toBe(12000);
  });

  it("exposes state variables in the tool schema", () => {
    const definition = JSON.parse(read("tools/unity_run_states.json"));
    const stateProperties = definition.parameters.properties.states.items.properties;

    expect(stateProperties.variables.type).toBe("string");
    expect(stateProperties.variables.description).toContain("state's start, update, and end");
    expect(definition.description).toContain("ctx.Global<T>");
    expect(definition.description).toContain("100000 estimated tokens");
    expect(definition.description).toContain("knowledge_read");
    expect(definition.description).toContain("skill/profiler.md");
    expect(read("knowledge/skill/profiler.md")).toContain("Unity Profiler Runtime Sampling");
    const runStatesBridge = read("locus_unity/Editor/LocusBridge.RunStates.cs");
    const profilerSkill = read("knowledge/skill/profiler.md");
    expect(runStatesBridge).toContain("StartProfiler(string name");
    expect(runStatesBridge).toContain("TryGetProfilerLastValue");
    expect(runStatesBridge).toContain("RecordProfilerSpike");
    expect(runStatesBridge).toContain("RecordProfilerSpikeTop");
    expect(runStatesBridge).toContain("locus.profiler.frame_hierarchy.v1");
    expect(runStatesBridge).toContain("sample_policy");
    expect(runStatesBridge).toContain("sample_rows=");
    expect(runStatesBridge).toContain("frame_span=");
    expect(runStatesBridge).toContain("session_frame_span");
    expect(runStatesBridge).toContain("inline_rows=");
    expect(runStatesBridge).toContain("value.ToString(\"G17\"");
    expect(runStatesBridge).toContain("profiler_summary_file");
    expect(profilerSkill).toContain("locus.profiler.samples_csv.v1");
    expect(profilerSkill).toContain("locus.profiler.summary.v1");
    expect(profilerSkill).toContain("sample_index,session_frame,unity_time_frame_count,profiler_frame_index,elapsed_ms");
    expect(profilerSkill).toContain("sample_rows=300 frame_span=299 unity_frame_span=299");
    expect(profilerSkill).toContain("\"sample_rows\": 300");
    expect(profilerSkill).toContain("ctx.RecordProfilerSpikeTop");
    expect(profilerSkill).toContain("ctx.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount)");
    expect(profilerSkill).toContain("ctx.SaveProfilerFrame(name, profilerFrameIndex, threadName, topCount, inlineRows)");
  });

  it("compiles unity_run_states before requesting an editor status change", () => {
    const agentSource = read("src-tauri/src/agent/instance/mod.rs");
    const runStatesStart = agentSource.indexOf("async fn execute_unity_run_states");
    const runStatesEnd = agentSource.indexOf("fn execute_unity_ref_search", runStatesStart);
    const runStatesSource = agentSource.slice(runStatesStart, runStatesEnd);

    expect(runStatesSource.indexOf("compile_run_states")).toBeGreaterThan(-1);
    expect(runStatesSource.indexOf("compile_run_states")).toBeLessThan(
      runStatesSource.indexOf("request_unity_editor_status_change_confirm"),
    );
    expect(runStatesSource).toContain("Compiling states");
    expect(runStatesSource).toContain("Compilation failed");
    expect(runStatesSource).toContain("Running state machine");
    expect(runStatesSource).toContain("Runtime failed");
    expect(runStatesSource).toContain("emit_tool_progress(");

    const executeStart = agentSource.indexOf("async fn execute_unity_execute");
    const executeEnd = agentSource.indexOf("async fn execute_unity_recompile", executeStart);
    expect(agentSource.slice(executeStart, executeEnd)).not.toContain("compile_run_states");

    expect(read("locus_unity/Editor/LocusBridge.cs")).toContain("compile_run_states");
    expect(read("locus_unity/Editor/LocusBridge.RunStates.cs")).toContain("HandleCompileRunStates");
  });

  it("wires the preview into completed tool calls and confirmation cards", () => {
    expect(read("src/components/ToolCallBlock.vue")).toContain("resolveToolBlockOverride");
    expect(read("src/components/tool-block-overrides/toolBlockOverrides.ts")).toContain("unity_run_states");
    expect(read("src/components/tool-block-overrides/UnityRunStatesToolBlock.vue")).toContain("<UnityRunStatesPreview");
    expect(read("src/components/tool-block-overrides/UnityRunStatesToolBlock.vue")).toContain("<UnityRunStatesOutputPreview");
    expect(read("src/components/tool-block-overrides/UnityRunStatesToolBlock.vue")).toContain("showFinalSections");
    expect(read("src/components/chat/ToolConfirmCard.vue")).toContain("<UnityRunStatesPreview");
  });

  it("keeps unity_run_states runtime progress above the collapsible tool details", () => {
    const source = read("src/components/tool-block-overrides/UnityRunStatesToolBlock.vue");
    const headerIndex = source.indexOf("class=\"tool-call-header");
    const progressIndex = source.indexOf("class=\"tool-call-progress-line");
    const detailIndex = source.indexOf("class=\"tool-call-detail");

    expect(headerIndex).toBeGreaterThanOrEqual(0);
    expect(progressIndex).toBeGreaterThanOrEqual(0);
    expect(detailIndex).toBeGreaterThanOrEqual(0);
    expect(headerIndex).toBeLessThan(progressIndex);
    expect(progressIndex).toBeLessThan(detailIndex);
    expect(source).toContain("<span v-if=\"headerSummary\" class=\"tool-call-summary\">");
    expect(source).toContain("const headerSummary = computed(() => runtimeProgressSummary.value)");
    expect(source).toContain("const toolProgress = computed(() => props.toolCall.status === \"running\" ? props.toolCall.progress : null)");
    expect(source).toContain("const toolProgressText = computed(() => {");
    expect(source).toContain("const showRuntimeProgressLine = computed(() => props.toolCall.status === \"running\" && Boolean(runtimePreview.value))");
    expect(source).toContain("const showToolProgressDots = computed(() => props.toolCall.status === \"running\" && Boolean(toolProgressText.value) && !runtimePreview.value)");
    expect(read("src/composables/unityRunStatesPreview.ts")).toContain("if (toolStatus === \"running\" && !hasOutput) return null;");
    expect(source).toContain("v-if=\"showRuntimeProgressLine\"");
    expect(source).toContain("class=\"tool-call-inline-dots\"");
    expect(source).toContain("const runtimePromptText = computed(() => runtimePreview.value?.promptText.trim() ?? \"\")");
    expect(source).toContain("const showRuntimePromptText = computed(() => props.toolCall.status === \"running\" && Boolean(runtimePromptText.value))");
    expect(source).toContain("class=\"unity-run-prompt-text ui-select-text\"");
    expect(source).toContain("const showRuntimePrintText = computed(() => props.toolCall.status === \"running\" && hasPrints.value)");
    expect(source).toContain("const showRuntimePrintFallback = computed(() => Boolean(runtimePreview.value) && !showRuntimePrintText.value)");
    expect(source).toContain("v-if=\"showRuntimePrintText\"");
    expect(source).toContain("v-else-if=\"showRuntimePrintFallback\"");
    expect(source).not.toContain("tool.unityRunStates.currentState");
    expect(source).not.toContain("tool.unityRunStates.userPrompt");
    expect(source).not.toContain("unity-run-progress-summary");
    expect(source).toContain("v-if=\"infoExpanded && hasInfoDetail\"");
    expect(source).toContain("const isFramed = computed(() => infoExpanded.value || showRuntimeProgressLine.value)");
    expect(source).toContain("'is-framed': isFramed");
    expect(source).toContain("const infoExpanded = ref(false)");
    expect(source).not.toContain("hide-prints");
    expect(source).not.toContain("collapseTimer");
    expect(source).not.toContain("1400");
    expect(source).toContain("class=\"unity-tool-call-block unity-run-tool-block\"");
    expect(source).not.toContain("class=\"tool-call-block unity-run-tool-block\"");
    expect(source).toMatch(/\.unity-tool-call-block\s*\{[\s\S]*border:\s*0/);
    expect(source).toMatch(/\.unity-tool-call-block\s*\{[\s\S]*border-radius:\s*0/);
    expect(source).toMatch(/\.unity-tool-call-block\s*\{[\s\S]*overflow:\s*visible/);
    expect(source).toMatch(/\.unity-tool-call-block\.is-framed\s*\{[\s\S]*border:\s*1px solid color-mix\(in srgb, #8b7cf6 46%, var\(--border-color\)\)/);
    expect(source).toMatch(/\.tool-call-detail\s*\{[\s\S]*padding:\s*6px 2px 0 20px/);
    expect(source).toMatch(/\.tool-call-progress-line\s*\{[\s\S]*padding:\s*5px 2px 0 20px/);
    expect(source).toMatch(/\.tool-call-progress-line\s*\{[\s\S]*border-top:\s*1px solid color-mix\(in srgb, var\(--border-color\) 58%, transparent\)/);
    expect(source).toMatch(/\.unity-run-progress\s*\{[\s\S]*background:\s*transparent/);
  });
});
