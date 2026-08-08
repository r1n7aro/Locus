//! Editor "flavor" abstraction: standard Unity vs. Tuanjie (团结引擎, the Unity
//! China fork). Tuanjie ships the same engine with a renamed executable
//! (`Tuanjie.exe`), a renamed engine PDB (`Tuanjie_x64.pdb`) and a separate Hub,
//! but keeps Unity's window class (`UnityContainerWndClass`), command-line flags
//! (`-projectPath`, `-useHub`, …) and engine symbol names (verified: the
//! `MonoManager::*` reload symbols and `IsApplicationActive*` are byte-identical
//! in `Tuanjie_x64.pdb`).
//!
//! Every helper here is ADDITIVE. Unity is always listed first in [`EditorFlavor::ALL`]
//! and its literals are returned unchanged, so standard-Unity behavior is
//! byte-for-byte identical to before this module existed. Tuanjie names are only
//! ever *also* accepted or *also* tried — never substituted for Unity's. This is
//! a hard requirement: nothing in here may change what an existing Unity install
//! resolves to.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorFlavor {
    Unity,
    Tuanjie,
}

impl EditorFlavor {
    /// Unity first so it always wins on standard installs; Tuanjie is only
    /// reached when no Unity candidate matched.
    pub(crate) const ALL: [EditorFlavor; 2] = [EditorFlavor::Unity, EditorFlavor::Tuanjie];

    /// Editor executable file name as installed on disk / launched.
    pub(crate) fn editor_exe_file_name(self) -> &'static str {
        match self {
            EditorFlavor::Unity => "Unity.exe",
            EditorFlavor::Tuanjie => "Tuanjie.exe",
        }
    }

    /// Lower-cased process image name, for case-insensitive process matching.
    pub(crate) fn process_image_name(self) -> &'static str {
        match self {
            EditorFlavor::Unity => "unity.exe",
            EditorFlavor::Tuanjie => "tuanjie.exe",
        }
    }

    /// In-process engine module in DLL form (preferred over the EXE when both
    /// are present).
    pub(crate) fn engine_module_dll(self) -> &'static str {
        match self {
            EditorFlavor::Unity => "Unity.dll",
            EditorFlavor::Tuanjie => "Tuanjie.dll",
        }
    }

    /// EXE that statically links the engine (the fallback engine module).
    pub(crate) fn engine_module_exe(self) -> &'static str {
        self.editor_exe_file_name()
    }

    /// Engine PDB file name shipped next to the editor executable.
    pub(crate) fn engine_pdb_name(self) -> &'static str {
        match self {
            EditorFlavor::Unity => "unity_x64.pdb",
            EditorFlavor::Tuanjie => "Tuanjie_x64.pdb",
        }
    }

    /// Token present in the editor's main-window title.
    pub(crate) fn window_title_token(self) -> &'static str {
        match self {
            EditorFlavor::Unity => "Unity",
            EditorFlavor::Tuanjie => "Tuanjie",
        }
    }
}

/// True when `image_name` (any case) is a known editor process image
/// (`Unity.exe` or `Tuanjie.exe`).
pub(crate) fn is_editor_process_image(image_name: &str) -> bool {
    EditorFlavor::ALL
        .iter()
        .any(|flavor| image_name.eq_ignore_ascii_case(flavor.process_image_name()))
}

/// True when `module_name` (any case) is a known in-process engine DLL.
pub(crate) fn is_engine_module_dll(module_name: &str) -> bool {
    EditorFlavor::ALL
        .iter()
        .any(|flavor| module_name.eq_ignore_ascii_case(flavor.engine_module_dll()))
}

/// True when `module_name` (any case) is a known editor EXE that links the engine.
pub(crate) fn is_engine_module_exe(module_name: &str) -> bool {
    EditorFlavor::ALL
        .iter()
        .any(|flavor| module_name.eq_ignore_ascii_case(flavor.engine_module_exe()))
}

/// True when `normalized_lower_path` (already lower-cased, back-slash separated)
/// ends with a known editor executable file name. Used to guard against PID
/// reuse when an `EditorInstance.json` omits `app_path`.
pub(crate) fn path_ends_with_editor_exe(normalized_lower_path: &str) -> bool {
    EditorFlavor::ALL.iter().any(|flavor| {
        normalized_lower_path.ends_with(&format!("\\{}", flavor.process_image_name()))
    })
}

/// Engine PDB file name for the engine module identified by `module_file_name`
/// (e.g. `Unity.exe`, `Unity.dll`, `Tuanjie.exe`). Anything not recognizably
/// Tuanjie returns Unity's exact PDB name, so the Unity path is unchanged.
pub(crate) fn engine_pdb_name_for_module(module_file_name: &str) -> &'static str {
    let stem = Path::new(module_file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if stem.eq_ignore_ascii_case("Tuanjie") {
        EditorFlavor::Tuanjie.engine_pdb_name()
    } else {
        EditorFlavor::Unity.engine_pdb_name()
    }
}

/// True when an editor main-window title belongs to a known editor flavor
/// (a Unity- or Tuanjie-tagged "… Editor …" title).
pub(crate) fn is_editor_window_title(title: &str) -> bool {
    title.contains("Editor")
        && EditorFlavor::ALL
            .iter()
            .any(|flavor| title.contains(flavor.window_title_token()))
}

/// True for a Tuanjie editor version string of the form `<n>.<n>.<n>t<n>`.
/// Tuanjie uses a `tN` release-type suffix (e.g. `2022.3.62t10`) where Unity
/// uses `fN`/`aN`/`bN`.
/// Requires a digit on both sides of the `t` so a stray `t` elsewhere in the
/// string can never be mistaken for the Tuanjie release-type marker.
pub(crate) fn is_tuanjie_version(version: &str) -> bool {
    let bytes = version.trim().as_bytes();
    for index in 1..bytes.len() {
        let current = bytes[index];
        if current != b't' && current != b'T' {
            continue;
        }
        let prev_is_digit = bytes[index - 1].is_ascii_digit();
        let next_is_digit = bytes
            .get(index + 1)
            .map(u8::is_ascii_digit)
            .unwrap_or(false);
        if prev_is_digit && next_is_digit {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_literals_are_unchanged() {
        assert_eq!(EditorFlavor::Unity.editor_exe_file_name(), "Unity.exe");
        assert_eq!(EditorFlavor::Unity.engine_pdb_name(), "unity_x64.pdb");
        // A non-Tuanjie module must keep resolving to Unity's exact PDB name.
        assert_eq!(engine_pdb_name_for_module("Unity.exe"), "unity_x64.pdb");
        assert_eq!(engine_pdb_name_for_module("Unity.dll"), "unity_x64.pdb");
        assert_eq!(engine_pdb_name_for_module(""), "unity_x64.pdb");
    }

    #[test]
    fn tuanjie_module_resolves_renamed_pdb() {
        assert_eq!(engine_pdb_name_for_module("Tuanjie.exe"), "Tuanjie_x64.pdb");
        assert_eq!(engine_pdb_name_for_module("tuanjie.exe"), "Tuanjie_x64.pdb");
    }

    #[test]
    fn editor_process_image_accepts_both_flavors() {
        assert!(is_editor_process_image("Unity.exe"));
        assert!(is_editor_process_image("unity.exe"));
        assert!(is_editor_process_image("Tuanjie.exe"));
        assert!(is_editor_process_image("TUANJIE.EXE"));
        assert!(!is_editor_process_image("notepad.exe"));
    }

    #[test]
    fn engine_module_matchers_cover_dll_and_exe() {
        assert!(is_engine_module_dll("Unity.dll"));
        assert!(is_engine_module_dll("Tuanjie.dll"));
        assert!(!is_engine_module_dll("Unity.exe"));
        assert!(is_engine_module_exe("Unity.exe"));
        assert!(is_engine_module_exe("Tuanjie.exe"));
    }

    #[test]
    fn path_guard_accepts_both_editor_exes() {
        assert!(path_ends_with_editor_exe(
            r"e:\2022.3.47f1\editor\unity.exe"
        ));
        assert!(path_ends_with_editor_exe(
            r"f:\tuanjie\2022.3.62t10\editor\tuanjie.exe"
        ));
        assert!(!path_ends_with_editor_exe(r"c:\windows\explorer.exe"));
    }

    #[test]
    fn window_title_matches_unity_and_tuanjie() {
        assert!(is_editor_window_title(
            "Game - SampleScene - Windows, Mac, Linux - Unity 2022.3.47f1"
        ));
        assert!(is_editor_window_title(
            "My project (1) tuanjie - SampleScene - Windows, Mac, Linux - Tuanjie Editor 1.9.2 <DX11>"
        ));
        assert!(!is_editor_window_title("Some Other Window"));
    }

    #[test]
    fn version_flavor_detection() {
        assert!(is_tuanjie_version("2022.3.62t10"));
        assert!(is_tuanjie_version("2022.3.2t3"));
        assert!(!is_tuanjie_version("2022.3.47f1"));
        assert!(!is_tuanjie_version("6000.3.14f1"));
        assert!(!is_tuanjie_version(""));
    }
}
