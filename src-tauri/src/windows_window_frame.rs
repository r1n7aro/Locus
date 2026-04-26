use tauri::Manager;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND},
};

const MAIN_WINDOW_LABEL: &str = "main";

pub fn restore_main_window_frame(app: &tauri::App) -> Result<(), String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Err(format!(
            "main webview window '{MAIN_WINDOW_LABEL}' was not found"
        ));
    };

    if let Err(error) = window.set_shadow(true) {
        eprintln!("[Locus] warning: failed to enable main window shadow: {error}");
    }

    let hwnd = window
        .hwnd()
        .map_err(|error| format!("failed to read main window handle: {error}"))?;
    apply_win11_round_corners(hwnd);
    Ok(())
}

fn apply_win11_round_corners(hwnd: HWND) {
    let preference = DWMWCP_ROUND;
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&preference) as u32,
        )
    };

    if let Err(error) = result {
        eprintln!("[Locus] warning: failed to apply Windows window corner preference: {error}");
    }
}
