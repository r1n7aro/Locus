import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("RichChatInput file drop state", () => {
  it("shows a composer drop state while external files are dragged over the input", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const composer = read("src/components/chat/ChatComposer.vue");
    const service = read("src/services/unity.ts");
    const command = read("src-tauri/src/commands/unity_embed.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(richInput).toContain("const localFileDragActive = ref(false);");
    expect(richInput).toContain("let localFileDragDepth = 0;");
    expect(richInput).toContain("subscribeLocusFileDragState");
    expect(richInput).toContain("type LocusFileDragStatePayload");
    expect(richInput).toContain("LOCAL_FILE_DRAG_STATE_TTL_MS");
    expect(richInput).toContain("function handleLocusFileDragState(payload: LocusFileDragStatePayload)");
    expect(richInput).toContain("scheduleLocalFileDragStateExpiry()");
    expect(richInput).toContain("function isExternalFileDrag(event: DragEvent): boolean");
    expect(richInput).toContain('types.includes("Files")');
    expect(richInput).toContain("function handleLocalFileDragEnter(event: DragEvent)");
    expect(richInput).toContain("function handleLocalFileDragOver(event: DragEvent)");
    expect(richInput).toContain("function handleLocalFileDragLeave(event: DragEvent)");
    expect(richInput).toContain("function handleLocalFileDrop(event: DragEvent)");
    expect(richInput).toContain(':drop-active="localFileDragActive"');
    expect(richInput).toContain(':drop-label="t(\'chat.input.dropFileHint\')"');
    expect(richInput).toContain('@dragenter="handleLocalFileDragEnter"');
    expect(richInput).toContain('@dragover="handleLocalFileDragOver"');
    expect(richInput).toContain('@dragleave="handleLocalFileDragLeave"');
    expect(richInput).toContain('@drop="handleLocalFileDrop"');
    expect(richInput).toContain('document.addEventListener("drop", handleDocumentLocalFileDrop)');
    expect(richInput).toContain('window.addEventListener("blur", handleWindowLocalFileDragBlur)');

    expect(composer).toContain("dropActive?: boolean;");
    expect(composer).toContain("dropLabel?: string;");
    expect(composer).toContain("'is-drop-active': dropActive");
    expect(composer).toContain('class="chat-composer-drop-overlay"');
    expect(composer).toContain('class="chat-composer-drop-label"');
    expect(composer).toContain(".chat-composer.is-drop-active");
    expect(composer).toContain(".chat-composer-drop-overlay");
    expect(composer).toContain(".chat-composer-drop-label");

    expect(service).toContain("export interface LocusFileDragStatePayload");
    expect(service).toContain("export function subscribeLocusFileDragState");
    expect(service).toContain('runtime.subscribe<LocusFileDragStatePayload>("locus-file-drag-state", handler)');

    expect(command).toContain('const FILE_DRAG_STATE_EVENT: &str = "locus-file-drag-state";');
    expect(command).toContain("struct LocusFileDragStatePayload");
    expect(command).toContain("emit_locus_file_drag_state_to(window.app_handle(), window.label(), drag_event)");
    expect(command).toContain("emit_locus_file_drag_state_to(webview.app_handle(), webview.label(), drag_event)");
    expect(command).toContain('phase: "enter".to_string()');
    expect(command).toContain('phase: "drop".to_string()');

    expect(zh).toContain('"chat.input.dropFileHint": "拖入以引用文件"');
    expect(en).toContain('"chat.input.dropFileHint": "Drop to reference files"');
  });
});
