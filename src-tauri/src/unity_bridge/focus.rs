pub fn bring_unity_to_foreground() -> Option<isize> {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        #[allow(non_snake_case)]
        #[repr(C)]
        struct INPUT {
            r#type: u32,
            ki: KEYBDINPUT,
        }

        #[allow(non_snake_case)]
        #[repr(C)]
        struct KEYBDINPUT {
            wVk: u16,
            wScan: u16,
            dwFlags: u32,
            time: u32,
            dwExtraInfo: usize,
        }

        #[allow(non_snake_case)]
        extern "system" {
            fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> isize;
            fn EnumWindows(
                lpEnumFunc: extern "system" fn(hwnd: isize, lparam: isize) -> i32,
                lparam: isize,
            ) -> i32;
            fn GetWindowTextW(hWnd: isize, lpString: *mut u16, nMaxCount: i32) -> i32;
            fn SetForegroundWindow(hWnd: isize) -> i32;
            fn ShowWindow(hWnd: isize, nCmdShow: i32) -> i32;
            fn IsIconic(hWnd: isize) -> i32;
            fn GetForegroundWindow() -> isize;
            fn SendInput(cInputs: u32, pInputs: *const INPUT, cbSize: i32) -> u32;
        }

        const SW_RESTORE: i32 = 9;
        const INPUT_KEYBOARD: u32 = 1;
        const KEYEVENTF_KEYUP: u32 = 0x0002;
        const VK_MENU: u16 = 0x12; // Alt key

        let prev_hwnd = unsafe { GetForegroundWindow() };

        unsafe fn activate_hwnd(hwnd: isize) {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            let inputs = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    ki: KEYBDINPUT {
                        wVk: VK_MENU,
                        wScan: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    ki: KEYBDINPUT {
                        wVk: VK_MENU,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            ];
            SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            SetForegroundWindow(hwnd);
        }

        extern "system" fn enum_callback(hwnd: isize, _lparam: isize) -> i32 {
            unsafe {
                let mut title = [0u16; 512];
                let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
                if len > 0 {
                    let title_str = String::from_utf16_lossy(&title[..len as usize]);
                    if crate::unity_bridge::flavor::is_editor_window_title(&title_str) {
                        activate_hwnd(hwnd);
                        return 0;
                    }
                }
            }
            1
        }

        unsafe {
            let class_name: Vec<u16> = OsStr::new("UnityContainerWndClass")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let hwnd = FindWindowW(class_name.as_ptr(), std::ptr::null());
            if hwnd != 0 {
                activate_hwnd(hwnd);
            } else {
                EnumWindows(enum_callback, 0);
            }
        }

        if prev_hwnd != 0 {
            Some(prev_hwnd)
        } else {
            None
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

pub fn restore_foreground(_hwnd: isize) {
    #[cfg(target_os = "windows")]
    let hwnd = _hwnd;
    #[cfg(target_os = "windows")]
    {
        #[allow(non_snake_case)]
        #[repr(C)]
        struct INPUT {
            r#type: u32,
            ki: KEYBDINPUT,
        }

        #[allow(non_snake_case)]
        #[repr(C)]
        struct KEYBDINPUT {
            wVk: u16,
            wScan: u16,
            dwFlags: u32,
            time: u32,
            dwExtraInfo: usize,
        }

        #[allow(non_snake_case)]
        extern "system" {
            fn SetForegroundWindow(hWnd: isize) -> i32;
            fn ShowWindow(hWnd: isize, nCmdShow: i32) -> i32;
            fn IsIconic(hWnd: isize) -> i32;
            fn SendInput(cInputs: u32, pInputs: *const INPUT, cbSize: i32) -> u32;
        }
        const SW_RESTORE: i32 = 9;
        const INPUT_KEYBOARD: u32 = 1;
        const KEYEVENTF_KEYUP: u32 = 0x0002;
        const VK_MENU: u16 = 0x12;

        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            let inputs = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    ki: KEYBDINPUT {
                        wVk: VK_MENU,
                        wScan: 0,
                        dwFlags: 0,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    ki: KEYBDINPUT {
                        wVk: VK_MENU,
                        wScan: 0,
                        dwFlags: KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            ];
            SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            SetForegroundWindow(hwnd);
        }
    }
}
