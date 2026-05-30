import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("View window negative coordinates", () => {
  it("preserves negative screen coordinates for secondary monitors", () => {
    const host = read("src/components/ViewHostWindow.vue");
    const runtime = read("src-tauri/src/view.rs");

    expect(host).toContain("x: Math.round(point.x - VIEW_HOST_DETACH_OFFSET_X)");
    expect(host).toContain("y: Math.round(point.y - VIEW_HOST_DETACH_OFFSET_Y)");
    expect(host).not.toContain("Math.max(0, Math.round(point.x - VIEW_HOST_DETACH_OFFSET_X))");
    expect(host).not.toContain("Math.max(0, Math.round(point.y - VIEW_HOST_DETACH_OFFSET_Y))");

    expect(runtime).toContain(
      ".set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))",
    );
    expect(runtime).toContain("let x = request.x.round() as i32;");
    expect(runtime).toContain("let y = request.y.round() as i32;");
    expect(runtime).toContain(".position(request.x, request.y)");
    expect(runtime).toContain("(Some(x), Some(y)) => Some((x, y))");
    expect(runtime).not.toContain("x.max(0.0)");
    expect(runtime).not.toContain("y.max(0.0)");
    expect(runtime).not.toContain("request.x.max(0.0)");
    expect(runtime).not.toContain("request.y.max(0.0)");
  });
});
