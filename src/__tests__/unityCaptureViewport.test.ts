import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string): string {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity viewport capture", () => {
  it("derives Game and Scene capture areas from their visible viewports", () => {
    const source = read("locus_unity/Editor/LocusBridge.CaptureViewport.cs");

    expect(source).toContain('TryReadCaptureRectProperty(window, "viewInParent"');
    expect(source).toContain('TryReadCaptureRectProperty(window, "viewInWindow"');
    expect(source).toContain('"cameraViewVisualElement",');
    expect(source).toContain('"worldBound",');
    expect(source).toContain('TryReadCaptureRectProperty(window, "cameraViewport"');
    expect(source).toContain('captureArea = "game_viewport"');
    expect(source).toContain('captureArea = "scene_viewport"');
    expect(source).toContain("CaptureViewportLayoutRetryCount = 5");
    expect(source).toContain('captureArea = "window"');
    expect(source).not.toContain('captureArea = "window_fallback"');
  });

  it("converts target-window points to physical pixels and keeps Win32 DPI-aware", () => {
    const source = read("locus_unity/Editor/LocusBridge.CaptureViewport.cs");

    expect(source).toContain('"scaledPixelsPerPoint"');
    expect(source).toContain("PointsToCapturePixelRect(screenPoints, pixelsPerPoint)");
    expect(source).toContain("SetThreadDpiAwarenessContext(CapturePerMonitorAwareV2)");
    expect(source).toContain("DwmGetWindowAttribute(");
  });

  it("maps top-down Win32 rows into Unity's bottom-up texture storage", () => {
    const source = read("locus_unity/Editor/LocusBridge.CaptureViewport.cs");

    expect(source).toContain("biHeight = -height");
    expect(source).toContain("int sourceY = cropY + (height - 1 - textureY);");
    expect(source).toContain("int targetRow = textureY * width * 4;");
  });

  it("reads Unity's normally rendered Game View texture without taking window focus", () => {
    const source = read("locus_unity/Editor/LocusBridge.CaptureViewport.cs");

    expect(source).toContain('"m_RenderTexture"');
    expect(source).toContain("s_CaptureGameViewTextureField.GetValue(gameView)");
    expect(source).toContain("PrepareCaptureWindowForCapture(operation);");
    expect(source).toContain("RenderTexture.GetTemporary(");
    expect(source).not.toContain("RenderPlayModeViewCamerasInternal");
    expect(source).not.toContain("SetPlayModeViewSize");
    expect(source).not.toContain("window.Focus();");
    expect(source).not.toContain("EditorWindow.GetWindow(gameViewType)");
    expect(source).not.toContain("EditorWindow.GetWindow<SceneView>()");
  });

  it("keeps native fallback captures background-safe and restores dock selection", () => {
    const source = read("locus_unity/Editor/LocusBridge.CaptureViewport.cs");

    expect(source).toContain("SelectCaptureWindowWithoutFocus(operation.window)");
    expect(source).toContain('FindCaptureInstanceProperty(\n                    host.GetType(),\n                    "selected")');
    expect(source).toContain("RestoreCaptureWindowSelection(operation);");
    expect(source).toContain("operation.useScreenCapture = true;");
    expect(source).toContain(
      "The target Unity tab could not be selected for background capture.",
    );
  });

  it("downscales engine captures on the GPU before CPU readback", () => {
    const source = read("locus_unity/Editor/LocusBridge.CaptureViewport.cs");

    expect(source).toContain("CalculateCaptureOutputSize(");
    expect(source).toContain("Graphics.Blit(source, resized);");
    expect(source).toContain("texture = ReadCaptureRenderTexture(");
  });

  it("exposes one optional output-size control without screen-range parameters", () => {
    const definition = JSON.parse(read("tools/unity_capture_viewport.json"));
    const properties = definition.parameters.properties;

    expect(Object.keys(properties)).toEqual([
      "request_editor_status",
      "target",
      "window_title",
      "max_long_edge",
    ]);
    expect(definition.parameters.required).toEqual(["request_editor_status", "target"]);
    expect(properties.max_long_edge).toMatchObject({
      type: "integer",
      minimum: 0,
      maximum: 8192,
      default: 1280,
    });
  });

  it("keeps source and output dimensions explicit across Unity and Rust", () => {
    const types = read("locus_unity/Editor/LocusBridge.Types.cs");
    const bridge = read("src-tauri/src/unity_bridge/capture.rs");
    const executor = read("src-tauri/src/agent/instance/unity_capture.rs");

    for (const field of ["sourceWidth", "sourceHeight", "outputWidth", "outputHeight"]) {
      expect(types).toContain(`public int ${field};`);
    }
    expect(bridge).toContain("pub fn effective_source_width(&self) -> u32");
    expect(bridge).toContain("pub fn effective_output_width(&self) -> u32");
    expect(executor).toContain("format_unity_capture_output(");
    expect(executor).toContain("source_width,");
    expect(executor).toContain("output_width,");
    expect(executor).toContain("applied_max_long_edge,");
  });
});
