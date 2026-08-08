import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat centered layout", () => {
  it("keeps the session transcript and composer on the same bounded center column", () => {
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(chatView).toContain("--chat-workspace-content-max-width: 980px;");
    expect(chatView).toContain('<div class="chat-input-frame">');
    expect(chatView).toMatch(/\.input-area\s*\{[\s\S]*border-top:\s*0;[\s\S]*background:\s*transparent;/);
    expect(chatView).toMatch(/\.chat-input-frame\s*\{[\s\S]*width:\s*min\(100%, var\(--chat-workspace-content-max-width\)\);[\s\S]*margin:\s*0 auto;/);
    expect(chatView).not.toContain("background: color-mix(in srgb, var(--bg-color) 94%, var(--text-color) 6%);");
    expect(chatView).not.toContain("border: 1px solid color-mix(in srgb, var(--border-strong) 76%, var(--text-secondary) 24%);");
    expect(chatView).not.toContain(".chat-input-frame :deep(.chat-composer:not(.is-drop-active))");
    expect(chatView).not.toContain(".chat-input-frame :deep(.chat-composer:not(:focus-within):not(.is-drop-active))");
    expect(chatView).toMatch(/\.chat-input-frame :deep\(\.chat-composer:not\(\.is-compact\):not\(\.has-top-extension\)\)\s*\{\s*min-height:\s*104px;/);
    expect(chatView).toMatch(/\.chat-pending-stack\s*\{[\s\S]*width:\s*min\(100%, var\(--chat-workspace-content-max-width\)\);[\s\S]*margin-inline:\s*auto;/);
    expect(transcript).toContain("scrollbar-gutter: stable both-edges;");
    expect(transcript).toMatch(/\.chat-transcript-scroll\.is-session > \.chat-transcript-content\s*\{[\s\S]*width:\s*min\(100%, var\(--chat-workspace-content-max-width, 980px\)\);[\s\S]*margin-inline:\s*auto;/);
  });
});
