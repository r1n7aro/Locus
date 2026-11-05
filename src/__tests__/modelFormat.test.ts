import { describe, expect, it } from "vitest";
import { dataUrlToArrayBuffer, detectModelFormat, modelFormatFromPath } from "../components/model-preview/modelFormat";

const bytes = (text: string): ArrayBuffer => new TextEncoder().encode(text).buffer as ArrayBuffer;

describe("modelFormat", () => {
  it("maps previewable extensions regardless of case and directory", () => {
    expect(modelFormatFromPath("Assets/Arts/SmallHouse.glb")).toBe("glb");
    expect(modelFormatFromPath("C:\\Models\\Hero.FBX")).toBe("fbx");
    expect(modelFormatFromPath("scene.gltf")).toBeNull();
    expect(modelFormatFromPath("notes.glb.txt")).toBeNull();
    expect(modelFormatFromPath("glb")).toBeNull();
  });

  it("detects the format from content before trusting the hint", () => {
    expect(detectModelFormat(bytes("glTF\u0002\u0000\u0000\u0000"), "fbx")).toBe("glb");
    expect(detectModelFormat(bytes("Kaydara FBX Binary  \u0000"), "glb")).toBe("fbx");
    expect(detectModelFormat(bytes("; FBX 7.4.0 project file"), null)).toBe("fbx");
  });

  it("falls back to the hint when the content has no known signature", () => {
    expect(detectModelFormat(bytes("????"), "glb")).toBe("glb");
    expect(detectModelFormat(bytes("????"))).toBeNull();
    expect(detectModelFormat(new ArrayBuffer(0), null)).toBeNull();
  });

  it("decodes base64 and percent-encoded data URLs", () => {
    const base64 = `data:model/gltf-binary;base64,${btoa("glTF\u0000\u00ff")}`;
    expect(Array.from(new Uint8Array(dataUrlToArrayBuffer(base64)))).toEqual([103, 108, 84, 70, 0, 255]);
    expect(Array.from(new Uint8Array(dataUrlToArrayBuffer("data:text/plain,a%20b")))).toEqual([97, 32, 98]);
    expect(() => dataUrlToArrayBuffer("http://example.test/model.glb")).toThrow();
  });
});
