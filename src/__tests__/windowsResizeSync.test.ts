import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("native Windows resize sync", () => {
  it("keeps WebView2 bounds synchronized from native resize messages", () => {
    const lib = read("src-tauri/src/lib.rs");
    const cargo = read("src-tauri/Cargo.toml");
    const sync = read("src-tauri/src/windows_resize_sync.rs");
    const frame = read("src-tauri/src/windows_window_frame.rs");
    const tauriConfig = read("src-tauri/tauri.conf.json");

    expect(lib).toContain("mod windows_resize_sync;");
    expect(lib).toContain("windows_resize_sync::install_for_main_window(app)");
    expect(lib).toContain("mod windows_window_frame;");
    expect(lib).toContain("windows_window_frame::restore_main_window_frame(app)");
    expect(cargo).toContain('webview2-com = "0.38.2"');
    expect(cargo).toContain('"Win32_Graphics_Dwm"');
    expect(tauriConfig).toContain('"shadow": true');
    expect(frame).toContain(".set_shadow(true)");
    expect(frame).toContain("WINDOWS_11_MIN_BUILD: u32 = 22000");
    expect(frame).toContain("supports_dwm_corner_preference()");
    expect(frame).toContain("CurrentBuildNumber");
    expect(frame).toMatch(/if\s+!supports_dwm_corner_preference\(\)\s*\{\s*return;\s*\}/);
    expect(frame).toContain("DwmSetWindowAttribute");
    expect(frame).toContain("DWMWA_WINDOW_CORNER_PREFERENCE");
    expect(frame).toContain("DWMWCP_ROUND");
    expect(sync).toContain("WM_WINDOWPOSCHANGED");
    expect(sync).toContain("WM_WINDOWPOSCHANGING");
    expect(sync).not.toContain("WM_SIZING");
    expect(sync).toContain('NATIVE_CLIENT_SIZE_EVENT: &str = "locus-native-window-client-size"');
    expect(sync).toContain("struct NativeWindowClientSize");
    expect(sync).toContain("struct WindowClientMetrics");
    expect(sync).toContain("window_client_metrics");
    expect(sync).toContain("proposed_client_dimension(window_pos.cx, metrics.frame_width)");
    expect(sync).toContain("proposed_client_dimension(window_pos.cy, metrics.frame_height)");
    expect(sync).toContain("publish_native_client_size");
    expect(sync).toMatch(/\.emit\(\s*NATIVE_CLIENT_SIZE_EVENT/);
    expect(sync).toContain("publish_native_client_size(state, proposed_client_width, proposed_client_height)");
    expect(sync).toContain("resize_target_active");
    expect(sync).toContain("state.resize_target_left = proposed_left;");
    expect(sync).toContain("state.resize_target_top = proposed_top;");
    expect(sync).toContain("let child_x = 0;");
    expect(sync).toContain("let child_y = 0;");
    expect(sync).toContain("current_client_width.max(proposed_client_width)");
    expect(sync).toContain("current_client_height.max(proposed_client_height)");
    expect(sync).toContain("parent_rect.left - state.resize_target_left");
    expect(sync).toContain("sync_webview_bounds_at(state, child_x, child_y, child_width, child_height, false)");
    expect(sync).not.toContain("current_width - window_pos.cx");
    expect(sync).toContain("GetWindowRect(hwnd, &mut window_rect)");
    expect(sync).toContain("right_stable");
    expect(sync).toContain("left_stable");
    expect(sync).toContain("let width_changed = window_pos.cx != current_width;");
    expect(sync).toContain("let height_changed = window_pos.cy != current_height;");
    expect(sync).toContain("if !width_changed && !height_changed");
    expect(sync).toContain("let horizontal_resize = width_changed && (right_stable || left_stable);");
    expect(sync).toContain("let vertical_resize = height_changed && (bottom_stable || top_stable);");
    expect(sync).toMatch(
      /sync_webview_bounds_at\(\s*state,\s*0,\s*0,\s*current_client_width,\s*current_client_height,\s*true,?\s*\)/,
    );
    expect(sync).toContain("sync_from_client_rect(state.parent_hwnd, state, true)");
    expect(sync).toContain("sync_from_client_rect(state.parent_hwnd, state, force)");
    expect(sync).toContain("let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };");
    expect(sync).toContain("if state.live_resize");
    expect(sync).toContain("notify_parent_position_changed(state)");
    expect(sync).toContain("TAURI_DRAG_RESIZE_BORDERS");
    expect(sync).toContain("EnumChildWindows");
    expect(sync).toContain("state.controller.SetBounds(bounds)");
    expect(sync).toContain("SetWindowPos(");
    expect(sync).toContain("CHILD_SUBCLASS_ID");
    expect(sync).toContain("child_sync_subclass_proc");
    expect(sync).toContain("target_child_frame");
    expect(sync).toContain("parent_client_origin");
    expect(sync).toContain("ClientToScreen");
    expect(sync).toContain("clamp_changing_child_window_pos");
    expect(sync).toContain("clamp_child_window_to_parent_client");
    expect(sync).toContain("RemoveWindowSubclass(hwnd, Some(child_sync_subclass_proc), CHILD_SUBCLASS_ID)");
    expect(sync).toContain("SET_WINDOW_POS_FLAGS(window_pos.flags.0 & !SWP_NOMOVE.0 & !SWP_NOSIZE.0)");
    expect(sync).toContain("state.last_x == x");
    expect(sync).toContain("state.last_width == width");
    expect(sync).not.toContain("std::thread::sleep");
  });
});
