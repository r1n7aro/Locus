import { effectScope, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useDocumentFileSession } from "../composables/useDocumentFileSession";

interface Document { text: string; hash: string; editable: boolean }
const document = (text: string, hash = text): Document => ({ text, hash, editable: true });
const scopes: ReturnType<typeof effectScope>[] = [];

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function setup() {
  const key = ref("checkout-a:file-a");
  const read = vi.fn<() => Promise<Document>>().mockResolvedValue(document("base", "v1"));
  const write = vi.fn<(base: Document, draft: string) => Promise<Document>>()
    .mockImplementation(async (_, draft) => document(draft, "v2"));
  const scope = effectScope();
  scopes.push(scope);
  const session = scope.run(() => useDocumentFileSession({
    key: () => key.value,
    emptyDraft: () => "",
    read,
    write,
    toDraft: (value) => value.text,
    equals: (left, right) => left === right,
    canSave: (value) => value.editable,
    errorMessage: (error) => String(error),
  }))!;
  return { session, key, read, write, scope };
}

afterEach(() => { for (const scope of scopes.splice(0)) scope.stop(); });

describe("shared document file session", () => {
  it("preserves raw whitespace, keeps renderer input stable while typing and skips clean saves", async () => {
    const { session, read, write } = setup();
    read.mockResolvedValue(document("001, ,\r\n\r\n"));
    await session.load();
    await expect(session.save()).resolves.toBe(true);
    expect(write).not.toHaveBeenCalled();
    session.updateDraft("001,  ,\r\n\r\n");
    expect(session.dirty.value).toBe(true);
    expect(session.modelDraft.value).toBe("001, ,\r\n\r\n");
    await expect(session.save()).resolves.toBe(true);
    expect(session.draft.value).toBe("001,  ,\r\n\r\n");
    expect(session.modelDraft.value).toBe(session.draft.value);
  });

  it("retains typing during save, updates the revision and requires another save before closing", async () => {
    const { session, write } = setup();
    await session.load();
    session.updateDraft("submitted");
    const pending = deferred<Document>();
    write.mockReturnValueOnce(pending.promise);
    const save = session.save();
    await expect(session.save()).resolves.toBe(false);
    expect(write).toHaveBeenCalledTimes(1);
    session.updateDraft("submitted and continued");
    pending.resolve(document("submitted", "v2"));
    await expect(save).resolves.toBe(false);
    expect(session.document.value?.hash).toBe("v2");
    expect(session.draft.value).toBe("submitted and continued");
    expect(session.modelDraft.value).toBe(session.draft.value);
    expect(session.dirty.value).toBe(true);
    await expect(session.save()).resolves.toBe(true);
    expect(write.mock.calls[1]?.[0].hash).toBe("v2");
  });

  it.each(["success", "failure"])("ignores a late save %s after switching files", async (outcome) => {
    const { session, key, read, write } = setup();
    await session.load();
    session.updateDraft("file-a draft");
    const pending = deferred<Document>();
    write.mockReturnValueOnce(pending.promise);
    const save = session.save();
    key.value = "checkout-b:file-b";
    read.mockResolvedValue(document("file-b", "b1"));
    await session.load();
    session.updateDraft("file-b draft");
    if (outcome === "success") pending.resolve(document("file-a draft", "a2"));
    else pending.reject(new Error("old write failed"));
    await expect(save).resolves.toBe(false);
    expect(session.document.value?.hash).toBe("b1");
    expect(session.draft.value).toBe("file-b draft");
    expect(session.error.value).toBe("");
    expect(session.saving.value).toBe(false);
  });

  it("ignores stale loads even when the resource changes back to the same key", async () => {
    const { session, key, read } = setup();
    const oldA = deferred<Document>();
    read.mockReturnValueOnce(oldA.promise);
    const firstLoad = session.load();
    key.value = "file-b";
    await session.load();
    key.value = "checkout-a:file-a";
    read.mockResolvedValue(document("new A"));
    await session.load();
    oldA.resolve(document("old A"));
    await expect(firstLoad).resolves.toBe(false);
    expect(session.draft.value).toBe("new A");
  });

  it("does not silently accept a new disk baseline if typing begins during reload", async () => {
    const { session, read } = setup();
    await session.load();
    const pending = deferred<Document>();
    read.mockReturnValueOnce(pending.promise);
    const reload = session.load({ keepCurrent: true });
    session.updateDraft("local");
    pending.resolve(document("remote", "remote-hash"));
    await expect(reload).resolves.toBe(false);
    expect(session.document.value?.hash).toBe("v1");
    expect(session.draft.value).toBe("local");
    expect(session.dirty.value).toBe(true);
  });

  it("supports explicit local/disk conflict choices without sharing their policy with renderers", async () => {
    const { session, read, write } = setup();
    await session.load();
    session.updateDraft("local");
    read.mockResolvedValue(document("remote", "remote-hash"));
    await expect(session.load({ keepCurrent: true, keepDraft: true })).resolves.toBe(true);
    expect(session.draft.value).toBe("local");
    expect(session.modelDraft.value).toBe("local");
    await session.save();
    expect(write.mock.calls[0]?.[0].hash).toBe("remote-hash");
    session.updateDraft("another local edit");
    await session.load({ keepCurrent: true });
    expect(session.draft.value).toBe("remote");
    expect(session.dirty.value).toBe(false);
  });

  it("retains the baseline when an adapter starts editing a companion during reload", async () => {
    const { session, read } = setup();
    await session.load();
    const pending = deferred<Document>();
    read.mockReturnValueOnce(pending.promise);
    let editing = false;
    const reload = session.load({ keepCurrent: true, canApply: () => !editing });
    editing = true;
    pending.resolve(document("remote", "remote-hash"));
    await expect(reload).resolves.toBe(false);
    expect(session.document.value?.hash).toBe("v1");
    expect(session.draft.value).toBe("base");
  });

  it("retains the editable local document when the disk version becomes unsupported", async () => {
    const { session, read } = setup();
    await session.load();
    session.updateDraft("local");
    read.mockResolvedValue({ text: "", hash: "binary", editable: false });
    await expect(session.load({ keepCurrent: true, keepDraft: true })).resolves.toBe(false);
    expect(session.document.value?.hash).toBe("v1");
    expect(session.draft.value).toBe("local");
  });

  it("retains the draft and baseline after a rejected write, and blocks read-only saves", async () => {
    const { session, read, write } = setup();
    await session.load();
    session.updateDraft("local");
    write.mockRejectedValueOnce(new Error("content changed"));
    await expect(session.save()).resolves.toBe(false);
    expect(session.error.value).toContain("content changed");
    expect(session.document.value?.hash).toBe("v1");
    expect(session.dirty.value).toBe(true);
    read.mockResolvedValue({ ...document("read only"), editable: false });
    await session.load();
    session.updateDraft("modified");
    await expect(session.save()).resolves.toBe(false);
    expect(write).toHaveBeenCalledTimes(1);
  });

  it("ignores results after the owner is disposed", async () => {
    const { session, read, scope } = setup();
    const pending = deferred<Document>();
    read.mockReturnValueOnce(pending.promise);
    const load = session.load();
    scope.stop();
    pending.resolve(document("late"));
    await expect(load).resolves.toBe(false);
    expect(session.document.value).toBeNull();
  });
});
