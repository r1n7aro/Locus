import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("RichChatInput image attachment layout", () => {
  it("keeps pasted image attachments in normal composer flow", () => {
    const source = read("src/components/chat/RichChatInput.vue");
    const composer = read("src/components/chat/ChatComposer.vue");
    const chatView = read("src/components/ChatView.vue");
    const embeddedPane = read("src/components/chat/EmbeddedChatPane.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain('class="composer-attachment-list"');
    expect(source).toContain('class="image-attachment-thumb-button ui-select-none"');
    expect(source).toContain('@click="openImagePreview(index)"');
    expect(source).toContain("const previewImageIndex = ref<number | null>(null);");
    expect(source).toContain('class="image-preview-overlay"');
    expect(source).toContain('class="image-preview-dialog"');
    expect(source).toContain(':src="previewImageSrc"');
    expect(source).toContain(':show-header="hasHeaderContent"');
    expect(source).toContain(':extend-top="hasTopAttachments"');
    expect(source).toContain("const hasTopAttachments = computed(() =>");
    expect(source).toMatch(/<template #overlay>[\s\S]*class="composer-attachment-list"/);
    expect(source).toContain('const file = item.kind === "file" ? item.getAsFile() : null;');
    expect(source).toContain("imageAttachments.value.map(({ data, mimeType }) => ({ data, mimeType }))");
    expect(source).not.toContain('class="image-preview-bar"');
    expect(source).toContain("flex-wrap: nowrap;");
    expect(source).toContain("overflow-x: auto;");
    expect(source).toContain("height: 28px;");
    expect(source).not.toContain('class="asset-ref-attachment-list"');
    expect(source).not.toContain('class="image-attachment-list"');
    expect(source).toMatch(/\.image-preview-overlay \{[\s\S]*position: fixed;[\s\S]*z-index: 9999;/);
    expect(source).toMatch(/\.image-preview-dialog \{[\s\S]*max-width: min\(76vw, 920px\);[\s\S]*max-height: min\(78vh, 720px\);/);
    expect(source).not.toContain(".image-attachment-item:hover .image-attachment-preview");
    expect(source).not.toContain('class="image-attachment-name"');
    expect(composer).toContain("showHeader?: boolean | null;");
    expect(composer).toContain("extendTop?: boolean;");
    expect(composer).toContain("const hasOverlay = computed(() => props.extendTop && !!slots.overlay);");
    expect(composer).toContain("const hasHeader = computed(() => props.showHeader ?? !!slots.header);");
    expect(composer).toContain('class="chat-composer-overlay"');
    expect(composer).toContain("'has-top-extension': extendTop");
    expect(composer).toMatch(/\.chat-composer-overlay \{[\s\S]*position: relative;[\s\S]*min-height: 30px;/);
    expect(composer).not.toContain("margin-top: -30px;");
    expect(composer).not.toContain("padding-top: 40px;");
    expect(chatView).not.toContain("composerHasImageAttachments");
    expect(chatView).not.toContain("has-composer-attachments");
    expect(chatView).not.toContain(":deep(.chat-composer-overlay)");
    expect(embeddedPane).not.toContain("composerHasImageAttachments");
    expect(embeddedPane).not.toContain("has-composer-attachments");
    expect(zh).toContain('"chat.paste.previewImage": "预览图片"');
    expect(en).toContain('"chat.paste.previewImage": "Preview image"');
  });
});
