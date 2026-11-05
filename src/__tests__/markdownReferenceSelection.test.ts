// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { injectFileRefs, injectWorkspaceMentions } from "../composables/markdownInject";
import {
  handleMarkdownReferenceCopy,
  handleMarkdownReferenceSelectStart,
  selectableMarkdownReference,
  serializeMarkdownReferenceSelection,
} from "../composables/markdownReferenceSelection";

function mountMarkdown(html: string): HTMLElement {
  const root = document.createElement("div");
  root.className = "markdown-body";
  root.innerHTML = html;
  document.body.append(root);
  return root;
}

function textNodeContaining(root: Node, text: string): Text {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if ((node as Text).data.includes(text)) return node as Text;
  }
  throw new Error(`missing text ${text}`);
}

afterEach(() => {
  document.getSelection()?.removeAllRanges();
  document.body.innerHTML = "";
});

describe("markdown reference selection", () => {
  it("copies references inside a selection as their full paths", () => {
    const root = mountMarkdown(`<p>${injectFileRefs("Edit src/components/Panel.vue:12 now")}</p>`);
    const label = root.querySelector(".md-ref-label")!;
    expect(label.textContent).toBe("Panel.vue:12");

    const start = textNodeContaining(root, "Edit");
    const end = textNodeContaining(root, " now");
    document.getSelection()!.setBaseAndExtent(start, 0, end, end.data.length);

    const clipboard = serializeMarkdownReferenceSelection(document);
    expect(clipboard?.text).toBe("Edit src/components/Panel.vue:12 now");
    expect(clipboard?.html).toContain("src/components/Panel.vue:12");
    expect(clipboard?.html).not.toContain("md-file-ref");
    expect(label.textContent).toBe("Panel.vue:12");
  });

  it("treats a reference partly inside the selection as selected whole", () => {
    const root = mountMarkdown(`<p>${injectWorkspaceMentions("See @src/core/Engine.ts and more")}</p>`);
    const reference = root.querySelector<HTMLElement>(".md-workspace-ref")!;
    const label = [...reference.childNodes].find((node) => node.nodeType === Node.TEXT_NODE) as Text;
    const start = textNodeContaining(root, "See");
    document.getSelection()!.setBaseAndExtent(start, 0, label, 3);

    const clipboard = serializeMarkdownReferenceSelection(document);
    expect(clipboard?.text).toBe("See src/core/Engine.ts");

    const selection = document.getSelection()!;
    expect(selection.focusNode).toBe(label);
    expect(selection.focusOffset).toBe(3);
    expect(label.data).toBe("Engine.ts");
  });

  it("leaves selections without references to the default copy", () => {
    const root = mountMarkdown("<p>Plain text only</p>");
    const text = textNodeContaining(root, "Plain");
    document.getSelection()!.setBaseAndExtent(text, 0, text, 5);
    expect(serializeMarkdownReferenceSelection(document)).toBeNull();
  });

  it("writes the clipboard and cancels the default copy", () => {
    const root = mountMarkdown(`<p>${injectFileRefs("Open utils/helpers.ts")}</p>`);
    const start = textNodeContaining(root, "Open");
    document.getSelection()!.setBaseAndExtent(start, 0, root.firstChild!, root.firstChild!.childNodes.length);

    const data = new Map<string, string>();
    let prevented = false;
    handleMarkdownReferenceCopy({
      target: root,
      defaultPrevented: false,
      clipboardData: { setData: (type: string, value: string) => data.set(type, value) },
      preventDefault: () => { prevented = true; },
    } as unknown as ClipboardEvent);

    expect(prevented).toBe(true);
    expect(data.get("text/plain")).toBe("Open utils/helpers.ts");
  });

  it("keeps a press on an inline reference from starting a text selection", () => {
    const root = mountMarkdown(`<p>${injectFileRefs("Open utils/helpers.ts")}</p>`);
    const labelText = root.querySelector(".md-ref-label")!.firstChild!;

    let prevented = false;
    handleMarkdownReferenceSelectStart({ target: labelText, preventDefault: () => { prevented = true; } } as unknown as Event);
    expect(prevented).toBe(true);

    prevented = false;
    handleMarkdownReferenceSelectStart({ target: textNodeContaining(root, "Open"), preventDefault: () => { prevented = true; } } as unknown as Event);
    expect(prevented).toBe(false);
  });

  it("does not treat block-level Unity previews as inline references", () => {
    const root = mountMarkdown('<span class="md-file-ref md-unity-object-ref" data-md-unity-level="inspector" title="Assets/A.prefab"><input></span>');
    expect(selectableMarkdownReference(root.querySelector("input"))).toBeNull();
  });
});
