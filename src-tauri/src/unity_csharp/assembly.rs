//! Which Unity assembly a script compiles into.
//!
//! Unity assigns a script to the assembly of the nearest ancestor folder
//! holding an `.asmdef` (or an `.asmref` pointing at one). Without one,
//! scripts under `Assets/` land in the predefined assemblies:
//!
//! | location                                                  | assembly                         |
//! |-----------------------------------------------------------|----------------------------------|
//! | top-level `Plugins/`, `Standard Assets/`, `Pro Standard Assets/` | `Assembly-CSharp-firstpass`  |
//! | an `Editor/` folder inside one of those                   | `Assembly-CSharp-Editor-firstpass` |
//! | any other `Editor/` folder                                | `Assembly-CSharp-Editor`         |
//! | everything else                                           | `Assembly-CSharp`                |
//!
//! Package scripts outside any asmdef are not compiled at all. The assembly
//! name is part of a `[SerializeReference]` instance's persisted identity
//! (`asm:`), so moving a script into another asmdef breaks those instances
//! unless `[MovedFrom(sourceAssembly: …)]` covers it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const ASSEMBLY_CSHARP: &str = "Assembly-CSharp";
pub const ASSEMBLY_CSHARP_FIRSTPASS: &str = "Assembly-CSharp-firstpass";
pub const ASSEMBLY_CSHARP_EDITOR: &str = "Assembly-CSharp-Editor";
pub const ASSEMBLY_CSHARP_EDITOR_FIRSTPASS: &str = "Assembly-CSharp-Editor-firstpass";

pub const PREDEFINED_ASSEMBLIES: [&str; 4] = [
    ASSEMBLY_CSHARP,
    ASSEMBLY_CSHARP_FIRSTPASS,
    ASSEMBLY_CSHARP_EDITOR,
    ASSEMBLY_CSHARP_EDITOR_FIRSTPASS,
];

const FIRSTPASS_ROOT_FOLDERS: [&str; 3] = ["plugins", "standard assets", "pro standard assets"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblySource {
    /// Workspace-relative path of the owning `.asmdef`.
    Asmdef(String),
    /// Workspace-relative path of the `.asmref` that redirected the folder.
    Asmref(String),
    Predefined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptAssembly {
    pub name: String,
    pub source: AssemblySource,
}

/// Resolves script → assembly with per-folder caching. `guid_to_path` maps a
/// 32-hex asset GUID to its workspace-relative path; it resolves
/// `.asmref` files that reference their asmdef by `GUID:…`.
pub struct AssemblyResolver<'a> {
    root: PathBuf,
    guid_to_path: Box<dyn Fn(&str) -> Option<String> + 'a>,
    dir_cache: HashMap<String, Option<ScriptAssembly>>,
    asmdef_names: HashMap<String, Option<String>>,
}

impl<'a> AssemblyResolver<'a> {
    pub fn new(root: &Path, guid_to_path: impl Fn(&str) -> Option<String> + 'a) -> Self {
        Self {
            root: root.to_path_buf(),
            guid_to_path: Box::new(guid_to_path),
            dir_cache: HashMap::new(),
            asmdef_names: HashMap::new(),
        }
    }

    /// Assembly for a workspace-relative script path, `None` when Unity would
    /// not compile it (outside `Assets/` / `Packages/`, or a package folder
    /// with no asmdef).
    pub fn assembly_for(&mut self, script_rel_path: &str) -> Option<ScriptAssembly> {
        let normalized = script_rel_path.replace('\\', "/");
        let components: Vec<&str> = normalized.split('/').filter(|c| !c.is_empty()).collect();
        if components.len() < 2 {
            return None;
        }
        let in_assets = components[0].eq_ignore_ascii_case("Assets");
        let in_packages = components[0].eq_ignore_ascii_case("Packages");
        if !in_assets && !in_packages {
            return None;
        }
        // Folder chain from the script's folder up to the scan boundary:
        // `Assets/` itself, or the package root `Packages/<name>/`.
        let boundary = if in_assets { 1 } else { 2 };
        let dir_count = components.len() - 1;
        if dir_count < boundary {
            return None;
        }
        for depth in (boundary..=dir_count).rev() {
            let dir = components[..depth].join("/");
            if let Some(found) = self.definition_in_dir(&dir) {
                return Some(found);
            }
        }
        if !in_assets {
            return None;
        }
        Some(ScriptAssembly {
            name: predefined_assembly(&components[1..dir_count]).to_string(),
            source: AssemblySource::Predefined,
        })
    }

    /// The `name` declared by an `.asmdef` file (workspace-relative).
    pub fn asmdef_name(&mut self, asmdef_rel_path: &str) -> Option<String> {
        if let Some(cached) = self.asmdef_names.get(asmdef_rel_path) {
            return cached.clone();
        }
        let name = read_json_string(&self.root.join(asmdef_rel_path), "name");
        self.asmdef_names
            .insert(asmdef_rel_path.to_string(), name.clone());
        name
    }

    fn definition_in_dir(&mut self, dir: &str) -> Option<ScriptAssembly> {
        if let Some(cached) = self.dir_cache.get(dir) {
            return cached.clone();
        }
        let found = self.read_definition_in_dir(dir);
        self.dir_cache.insert(dir.to_string(), found.clone());
        found
    }

    fn read_definition_in_dir(&mut self, dir: &str) -> Option<ScriptAssembly> {
        let entries = std::fs::read_dir(self.root.join(dir)).ok()?;
        let mut asmdef: Option<String> = None;
        let mut asmref: Option<String> = None;
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            let lower = file_name.to_ascii_lowercase();
            if lower.ends_with(".asmdef") && asmdef.is_none() {
                asmdef = Some(format!("{dir}/{file_name}"));
            } else if lower.ends_with(".asmref") && asmref.is_none() {
                asmref = Some(format!("{dir}/{file_name}"));
            }
        }
        if let Some(path) = asmdef {
            let name = self.asmdef_name(&path)?;
            return Some(ScriptAssembly {
                name,
                source: AssemblySource::Asmdef(path),
            });
        }
        let path = asmref?;
        let reference = read_json_string(&self.root.join(&path), "reference")?;
        let name = match reference.strip_prefix("GUID:") {
            Some(guid) => {
                let target = (self.guid_to_path)(&guid.trim().to_ascii_lowercase())?;
                self.asmdef_name(&target)?
            }
            None => reference,
        };
        Some(ScriptAssembly {
            name,
            source: AssemblySource::Asmref(path),
        })
    }
}

/// Predefined assembly for a script outside any asmdef. `dirs` are the
/// folders below `Assets/`, outermost first.
pub fn predefined_assembly(dirs: &[&str]) -> &'static str {
    let firstpass = dirs
        .first()
        .map(|first| FIRSTPASS_ROOT_FOLDERS.contains(&first.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    let editor = dirs.iter().any(|dir| dir.eq_ignore_ascii_case("Editor"));
    match (firstpass, editor) {
        (true, true) => ASSEMBLY_CSHARP_EDITOR_FIRSTPASS,
        (true, false) => ASSEMBLY_CSHARP_FIRSTPASS,
        (false, true) => ASSEMBLY_CSHARP_EDITOR,
        (false, false) => ASSEMBLY_CSHARP,
    }
}

fn read_json_string(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim_start_matches('\u{feff}');
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let found = value.get(key)?.as_str()?.trim();
    if found.is_empty() {
        None
    } else {
        Some(found.to_string())
    }
}
