// @vitest-environment jsdom
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { history, undo } from "@codemirror/commands";
import { afterEach, describe, expect, it, vi } from "vitest";
import { markdownImagePaste } from "../components/ui/markdown-editor/markdownImagePaste";

const mocks = vi.hoisted(() => ({ import: vi.fn() }));
vi.mock("../services/markdownImage", () => ({ importMarkdownImage: mocks.import, MARKDOWN_IMAGE_DIRECTORY_UNSET: "markdown.image_directory_unset" }));
const unset = { code: "markdown.image_directory_unset", message: "Choose a folder", retryable: false, severity: "error" };
let view: EditorView | null = null;
afterEach(() => { view?.destroy(); view = null; document.body.replaceChildren(); mocks.import.mockReset(); });
function setup(chooseFolder?: Parameters<typeof markdownImagePaste>[0]["chooseFolder"]) {
  const context = { workspaceRef: { checkoutId: "a" }, contentPath: "Docs/a.md", key: "a" };
  const onError = vi.fn();
  const root = document.createElement("div"); document.body.append(root);
  view = new EditorView({ parent: root, state: EditorState.create({ doc: "before after", selection: { anchor: 7 }, extensions: [history(), markdownImagePaste({ context: () => context, onError, chooseFolder })] }) });
  return { view, context, onError };
}
function paste(view: EditorView) {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: { files: [new File(["png"], "clipboard.png", { type: "image/png" })] } });
  view.contentDOM.dispatchEvent(event); return event;
}
async function flush() { for (let i = 0; i < 6; i++) await Promise.resolve(); }
describe("shared Markdown image import", () => {
  it("maps the original paste position through continued typing and supports undo", async () => {
    const { view } = setup();
    let resolve!: (image: { path: string; relativePath: string }) => void;
    mocks.import.mockReturnValue(new Promise(yes => { resolve = yes; }));
    expect(paste(view).defaultPrevented).toBe(true);
    expect(mocks.import).toHaveBeenCalledWith({ checkoutId: "a" }, "Docs/a.md", expect.any(File));
    view.dispatch({ changes: { from: 0, insert: "new " }, selection: { anchor: 0 } });
    resolve({ path: "Locus/attachments/images/a.png", relativePath: "../Locus/attachments/images/a.png" });
    await flush();
    expect(view.state.doc.toString()).toBe("new before \n\n![图片](../Locus/attachments/images/a.png)\n\nafter");
    expect(view.state.selection.main.head).toBe(0);
    expect(undo(view)).toBe(true);
    expect(view.state.doc.toString()).toBe("new before after");
  });
  it("never inserts into another document after an asynchronous import", async () => {
    const { view, context } = setup();
    let resolve!: (image: { path: string; relativePath: string }) => void;
    mocks.import.mockReturnValue(new Promise(yes => { resolve = yes; }));
    paste(view); context.key = "b";
    resolve({ path: "a.png", relativePath: "a.png" }); await flush();
    expect(view.state.doc.toString()).toBe("before after");
  });
  it("asks for the project's image folder once and retries the import", async () => {
    let answer!: (saved: boolean) => void;
    const chooseFolder = vi.fn(() => new Promise<boolean>(yes => { answer = yes; }));
    const { view, onError } = setup(chooseFolder);
    mocks.import.mockRejectedValueOnce(unset).mockResolvedValueOnce({ path: "img/a.png", relativePath: "../img/a.png" });
    paste(view); await flush();
    expect(chooseFolder).toHaveBeenCalledWith(expect.objectContaining({ workspaceRef: { checkoutId: "a" }, contentPath: "Docs/a.md" }));
    expect(view.state.doc.toString()).toBe("before after");
    answer(true); await flush();
    expect(mocks.import).toHaveBeenCalledTimes(2);
    expect(view.state.doc.toString()).toBe("before \n\n![图片](../img/a.png)\n\nafter");
    expect(onError).not.toHaveBeenCalled();
  });
  it("inserts nothing when choosing the folder is cancelled", async () => {
    const chooseFolder = vi.fn(async () => false);
    const { view, onError } = setup(chooseFolder);
    mocks.import.mockRejectedValue(unset);
    paste(view); await flush();
    expect(chooseFolder).toHaveBeenCalledOnce();
    expect(mocks.import).toHaveBeenCalledOnce();
    expect(view.state.doc.toString()).toBe("before after");
    expect(onError).not.toHaveBeenCalled();
  });
  it("reports failed imports without deleting the selected text", async () => {
    const { view, onError } = setup();
    view.dispatch({ selection: { anchor: 0, head: 6 } });
    mocks.import.mockRejectedValue(new Error("disk full"));
    paste(view); await flush();
    expect(view.state.doc.toString()).toBe("before after");
    expect(onError).toHaveBeenCalledOnce();
  });
});
