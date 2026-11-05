use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND},
};

const WINDOWS_11_MIN_BUILD: u32 = 22000;

/// Shadow and Windows 11 rounded corners for an undecorated application window.
pub fn apply_window_frame(window: &tauri::WebviewWindow) -> Result<(), String> {
    if let Err(error) = window.set_shadow(true) {
        eprintln!(
            "[Locus] warning: failed to enable shadow for window '{}': {error}",
            window.label()
        );
    }

    let hwnd = window
        .hwnd()
        .map_err(|error| format!("failed to read window handle: {error}"))?;
    apply_win11_round_corners(hwnd);
    Ok(())
}

fn apply_win11_round_corners(hwnd: HWND) {
    if !supports_dwm_corner_preference() {
        return;
    }

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

fn supports_dwm_corner_preference() -> bool {
    windows_build_number().is_some_and(|build| build >= WINDOWS_11_MIN_BUILD)
}

fn windows_build_number() -> Option<u32> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let current_version = hklm
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?;
    let build = current_version
        .get_value::<String, _>("CurrentBuildNumber")
        .ok()?;
    build.parse::<u32>().ok()
}
