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

  it("builds a runtime panel preview from state code and final output", () => {
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
    expect(running?.currentState).toBe("wait_player");
    expect(running?.promptText).toBe("press jump");
    expect(running?.printText).toContain("ready for jump");
    expect(running?.isFinal).toBe(false);

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
  });

  it("wires the preview into completed tool calls and confirmation cards", () => {
    expect(read("src/components/ToolCallBlock.vue")).toContain("resolveToolBlockOverride");
    expect(read("src/components/tool-block-overrides/toolBlockOverrides.ts")).toContain("unity_run_states");
    expect(read("src/components/tool-block-overrides/UnityRunStatesToolBlock.vue")).toContain("<UnityRunStatesPreview");
    expect(read("src/components/tool-block-overrides/UnityRunStatesToolBlock.vue")).toContain("<UnityRunStatesOutputPreview");
    expect(read("src/components/tool-block-overrides/UnityRunStatesToolBlock.vue")).toContain("showFinalSections");
    expect(read("src/components/chat/ToolConfirmCard.vue")).toContain("<UnityRunStatesPreview");
  });
});
