//! Unity project-file (`.sln`/`.csproj`) generation.
//!
//! Roslyn loads the workspace through MSBuild project files, but a fresh
//! Unity checkout has none until an IDE package inside the editor writes
//! them. Two generation channels, tried by `discover_project_target`:
//!
//! 1. A connected editor (Locus bridge) runs [`editor_sync_snippet`].
//! 2. No editor running: a one-shot headless `-batchmode` editor run.
//!    The `-executeMethod` entry point is carried by a temporary embedded
//!    package written into `Packages/` and removed afterwards; the project
//!    the package itself produced is stripped back out of the generated
//!    solution so no trace of it remains. (The batch session may record the
//!    package in `Packages/packages-lock.json`; Unity drops stale embedded
//!    entries on the next project open, and rewriting that file here would
//!    scramble its key order for a purely cosmetic win, so it is left alone.)
//!
//! Both channels share the same C# fallback chain: the configured external
//! code editor first (`CodeEditor.SyncAll`), then the known IDE packages'
//! `ProjectGeneration` via reflection (works even when no external editor is
//! configured), then the legacy `UnityEditor.SyncVS`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::unity_bridge::{self, UnityEditorProcessState};

/// Folder under `Packages/` for the throwaway entry-point package.
const TEMP_PACKAGE_FOLDER: &str = "com.locus.projectsync.tmp";
/// Assembly definition name; doubles as the entry-point class name and the
/// stem of the csproj Unity generates for the temporary package.
const TEMP_ASSEMBLY_NAME: &str = "LocusProjectFileSyncTemp";
/// First asset import of a large project can dominate this; the message on
/// timeout tells the user to open the project in Unity once instead.
const BATCH_SYNC_TIMEOUT: Duration = Duration::from_secs(900);
/// How much of the editor log to scan and quote when the run fails.
const LOG_TAIL_BYTES: usize = 16 * 1024;

/// C# statements implementing the generator fallback chain. Valid both as an
/// execute-code snippet body (local functions in a method body) and inlined
/// into the temporary package's `Run()`. Ends with `__locusSyncReport` in
/// scope describing what was attempted.
const SYNC_CHAIN_STATEMENTS: &str = r#"
var __locusSyncLog = new System.Text.StringBuilder();
bool __locusSynced = false;

bool __LocusTrySyncAll()
{
    var editor = Unity.CodeEditor.CodeEditor.CurrentEditor;
    if (editor == null)
    {
        __locusSyncLog.AppendLine("CodeEditor: no current editor");
        return false;
    }
    var editorName = editor.GetType().Name;
    if (editorName == "DefaultExternalCodeEditor")
    {
        __locusSyncLog.AppendLine("CodeEditor: default editor has no project generator");
        return false;
    }
    editor.SyncAll();
    __locusSyncLog.AppendLine("CodeEditor.SyncAll (" + editorName + "): ok");
    return true;
}

bool __LocusTryPackageGenerators()
{
    string[] generatorTypeNames =
    {
        "Microsoft.Unity.VisualStudio.Editor.ProjectGeneration, Unity.VisualStudio.Editor",
        "Packages.Rider.Editor.ProjectGeneration.ProjectGeneration, Unity.Rider.Editor",
        "VSCodeEditor.ProjectGeneration, Unity.VSCode.Editor",
    };
    foreach (var typeName in generatorTypeNames)
    {
        try
        {
            var generatorType = System.Type.GetType(typeName, false);
            if (generatorType == null)
                continue;
            var generator = System.Activator.CreateInstance(generatorType, true);
            var syncMethod = generatorType.GetMethod(
                "Sync",
                System.Reflection.BindingFlags.Public
                    | System.Reflection.BindingFlags.NonPublic
                    | System.Reflection.BindingFlags.Instance,
                null,
                System.Type.EmptyTypes,
                null);
            if (syncMethod == null)
            {
                __locusSyncLog.AppendLine(typeName + ": Sync() not found");
                continue;
            }
            syncMethod.Invoke(generator, null);
            __locusSyncLog.AppendLine(typeName + ": ok");
            return true;
        }
        catch (System.Exception e)
        {
            var inner = e.InnerException != null ? e.InnerException : e;
            __locusSyncLog.AppendLine(typeName + ": " + inner.Message);
        }
    }
    return false;
}

bool __LocusTrySyncVs()
{
    var syncVsType = System.Type.GetType("UnityEditor.SyncVS,UnityEditor", false);
    if (syncVsType == null)
    {
        __locusSyncLog.AppendLine("UnityEditor.SyncVS: not present");
        return false;
    }
    var syncSolution = syncVsType.GetMethod(
        "SyncSolution",
        System.Reflection.BindingFlags.Static
            | System.Reflection.BindingFlags.Public
            | System.Reflection.BindingFlags.NonPublic);
    if (syncSolution == null)
    {
        __locusSyncLog.AppendLine("SyncVS.SyncSolution: not found");
        return false;
    }
    syncSolution.Invoke(null, null);
    __locusSyncLog.AppendLine("SyncVS.SyncSolution: ok");
    return true;
}

try { __locusSynced = __LocusTrySyncAll(); }
catch (System.Exception e) { __locusSyncLog.AppendLine("CodeEditor.SyncAll: " + e.Message); }
if (!__locusSynced)
{
    try { __locusSynced = __LocusTryPackageGenerators(); }
    catch (System.Exception e) { __locusSyncLog.AppendLine("package generators: " + e.Message); }
}
if (!__locusSynced)
{
    try { __locusSynced = __LocusTrySyncVs(); }
    catch (System.Exception e) { __locusSyncLog.AppendLine("SyncVS: " + e.Message); }
}
var __locusSyncReport =
    (__locusSynced ? "synced" : "no project generator succeeded") + "\n" + __locusSyncLog;
"#;

/// Snippet for `unity_execute_code` against a connected editor.
pub fn editor_sync_snippet() -> String {
    [SYNC_CHAIN_STATEMENTS, "\nprint(__locusSyncReport);\n"].concat()
}

/// `-executeMethod` argument for the headless run.
fn execute_method() -> String {
    format!("{TEMP_ASSEMBLY_NAME}.Run")
}

/// Source of the temporary package's entry point.
fn batch_entry_point_source() -> String {
    [
        "// Temporary entry point written by Locus to generate .sln/.csproj in\n",
        "// batch mode. The package is deleted right after the run.\n",
        "public static class ",
        TEMP_ASSEMBLY_NAME,
        "\n{\n    public static void Run()\n    {\n",
        SYNC_CHAIN_STATEMENTS,
        "\n        UnityEngine.Debug.Log(\"[LocusProjectSync] \" + __locusSyncReport);\n",
        "    }\n}\n",
    ]
    .concat()
}

fn temp_package_dir(root: &Path) -> PathBuf {
    root.join("Packages").join(TEMP_PACKAGE_FOLDER)
}

/// Write the throwaway embedded package (auto-discovered by Unity, no
/// manifest.json change needed). The asmdef keeps the entry point in its own
/// assembly so user script compile errors cannot take it down.
fn write_temp_package(dir: &Path) -> Result<(), String> {
    let write = |name: &str, contents: String| -> Result<(), String> {
        std::fs::write(dir.join(name), contents)
            .map_err(|error| format!("Failed to write {}/{}: {}", dir.display(), name, error))
    };
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Failed to create {}: {}", dir.display(), error))?;
    write(
        "package.json",
        format!(
            "{{\n  \"name\": \"{TEMP_PACKAGE_FOLDER}\",\n  \"version\": \"1.0.0\",\n  \
             \"displayName\": \"Locus Project File Sync (temporary)\",\n  \
             \"description\": \"Written by Locus to generate .sln/.csproj in batch mode. Safe to delete.\"\n}}\n"
        ),
    )?;
    write(
        &format!("{TEMP_ASSEMBLY_NAME}.asmdef"),
        format!(
            "{{\n  \"name\": \"{TEMP_ASSEMBLY_NAME}\",\n  \"references\": [],\n  \
             \"includePlatforms\": [\"Editor\"],\n  \"excludePlatforms\": [],\n  \
             \"autoReferenced\": false\n}}\n"
        ),
    )?;
    write(&format!("{TEMP_ASSEMBLY_NAME}.cs"), batch_entry_point_source())
}

fn remove_temp_package(dir: &Path) {
    if dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(dir) {
            eprintln!(
                "[CsharpLsp] failed to remove temporary sync package {}: {}",
                dir.display(),
                error
            );
        }
    }
}

/// Drop the temporary package's own project from a generated solution:
/// its `Project(...) .. EndProject` block plus every line referencing its
/// GUID (configuration platforms, nested projects). Returns `None` when the
/// solution does not mention it.
fn strip_project_from_sln_text(sln: &str, project_file_name: &str) -> Option<String> {
    let needle = format!("\"{project_file_name}\"");
    let eol = if sln.contains("\r\n") { "\r\n" } else { "\n" };
    let had_trailing_newline = sln.ends_with('\n');

    let mut guid: Option<String> = None;
    for line in sln.lines() {
        if line.trim_start().starts_with("Project(") && line.contains(&needle) {
            let open = line.rfind('{')?;
            let close = line.rfind('}')?;
            if close > open {
                guid = Some(line[open..=close].to_string());
            }
            break;
        }
    }
    let guid = guid?;

    let mut kept: Vec<&str> = Vec::new();
    let mut in_project_block = false;
    for line in sln.lines() {
        if in_project_block {
            if line.trim() == "EndProject" {
                in_project_block = false;
            }
            continue;
        }
        if line.trim_start().starts_with("Project(") && line.contains(&needle) {
            in_project_block = true;
            continue;
        }
        if line.contains(&guid) {
            continue;
        }
        kept.push(line);
    }

    let mut result = kept.join(eol);
    if had_trailing_newline {
        result.push_str(eol);
    }
    Some(result)
}

/// Remove what the temporary package left in the generated output: its
/// csproj at the workspace root and its entries in every root solution.
/// Best-effort — generation already succeeded; leftovers only cost a load
/// warning, so failures are logged rather than propagated.
fn strip_temp_project_artifacts(root: &Path) {
    let csproj_name = format!("{TEMP_ASSEMBLY_NAME}.csproj");
    let csproj = root.join(&csproj_name);
    if csproj.exists() {
        if let Err(error) = std::fs::remove_file(&csproj) {
            eprintln!(
                "[CsharpLsp] failed to remove {}: {}",
                csproj.display(),
                error
            );
        }
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_sln = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sln"));
        if !is_sln || !path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(stripped) = strip_project_from_sln_text(&text, &csproj_name) {
            if let Err(error) = std::fs::write(&path, stripped) {
                eprintln!(
                    "[CsharpLsp] failed to rewrite {}: {}",
                    path.display(),
                    error
                );
            }
        }
    }
}

fn batch_log_path(root: &Path) -> PathBuf {
    let tag = blake3::hash(root.to_string_lossy().as_bytes())
        .to_hex()
        .chars()
        .take(12)
        .collect::<String>();
    std::env::temp_dir().join(format!("locus-unity-project-sync-{tag}.log"))
}

fn log_tail(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(LOG_TAIL_BYTES);
    String::from_utf8_lossy(&bytes[start..]).to_string()
}

/// Map a failed batch run to an actionable message using the editor log.
fn batch_failure_message(exit_code: Option<i32>, tail: &str) -> String {
    let lowered = tail.to_lowercase();
    if lowered.contains("license") && (lowered.contains("invalid") || lowered.contains("no valid")
        || lowered.contains("not activated") || lowered.contains("expired"))
    {
        return "Unity batch mode could not run because no active Unity license was found. \
                Sign in via Unity Hub (or activate a license), then retry."
            .to_string();
    }
    if lowered.contains("another unity instance") || lowered.contains("already open") {
        return "Unity refused the batch run because another editor instance has this project \
                open. Close it (or let the Locus bridge connect to it), then retry."
            .to_string();
    }
    let mut message = String::from(
        "Unity batch mode ran but produced no .sln/.csproj. Open the project in Unity once \
         with an external script editor configured (Edit > Preferences > External Tools), \
         then retry.",
    );
    if let Some(code) = exit_code {
        message.push_str(&format!(" (exit code {code})"));
    }
    let snippet = tail
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    if !snippet.is_empty() {
        message.push_str("\nEditor log tail:\n");
        message.push_str(&snippet);
    }
    message
}

fn generation_gate() -> &'static tokio::sync::Mutex<()> {
    static GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Generate project files by running the project's exact editor version
/// headless. Caller has established that the workspace is a Unity project
/// with no connected editor. Returns `Ok` only when `.sln`/`.csproj` exist
/// afterwards.
pub async fn generate_headless(root: &Path) -> Result<(), String> {
    let _gate = generation_gate().lock().await;
    // A concurrent attempt may have produced the files while we waited.
    if super::scan_project_target(root).is_some() {
        return Ok(());
    }

    let workspace = root.to_string_lossy().to_string();
    let probe = unity_bridge::query_current_project_editor_process(&workspace).await;
    if matches!(probe.state, UnityEditorProcessState::Running) {
        return Err(
            "A Unity editor has this project open but the Locus bridge is not connected, so \
             Locus can neither ask it to generate .sln/.csproj nor start a batch instance. \
             Install the Locus Unity plugin so the bridge connects, or use Unity's \
             'Assets > Open C# Project' menu once, then retry."
                .to_string(),
        );
    }

    let version = unity_bridge::read_project_unity_version(&workspace)?.ok_or_else(|| {
        "ProjectSettings/ProjectVersion.txt is missing, so the matching Unity editor version \
         cannot be determined."
            .to_string()
    })?;
    let editor = unity_bridge::resolve_unity_editor_executable(&version).map_err(|error| {
        format!(
            "{error}. Install this exact version via Unity Hub, or point \
             LOCUS_UNITY_EDITOR_PATH at the editor binary, then retry."
        )
    })?;

    let package_dir = temp_package_dir(root);
    write_temp_package(&package_dir)?;

    let log_file = batch_log_path(root);
    let _ = std::fs::remove_file(&log_file);

    eprintln!(
        "[CsharpLsp] generating project files via Unity batch mode: editor='{}', project='{}'",
        editor.display(),
        root.display()
    );

    let mut command = tokio::process::Command::new(&editor);
    command
        .arg("-batchmode")
        .arg("-nographics")
        .arg("-ignorecompilererrors")
        .arg("-quit")
        .arg("-projectPath")
        .arg(root)
        .arg("-executeMethod")
        .arg(execute_method())
        .arg("-logFile")
        .arg(&log_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);

    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let spawned = command.spawn().map_err(|error| {
        format!(
            "Failed to launch Unity Editor '{}' for batch generation: {}",
            editor.display(),
            error
        )
    });
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            remove_temp_package(&package_dir);
            return Err(error);
        }
    };

    let deadline = tokio::time::Instant::now() + BATCH_SYNC_TIMEOUT;
    let exit_code: Option<i32> = loop {
        tokio::select! {
            status = child.wait() => {
                break status.ok().and_then(|s| s.code());
            }
            _ = tokio::time::sleep_until(deadline) => {
                let _ = child.kill().await;
                remove_temp_package(&package_dir);
                return Err(format!(
                    "Unity batch project-file generation timed out after {} minutes (the first \
                     asset import of a large project can exceed this). Open the project in \
                     Unity once and let the import finish, then retry.",
                    BATCH_SYNC_TIMEOUT.as_secs() / 60
                ));
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if !super::is_enabled() {
                    let _ = child.kill().await;
                    remove_temp_package(&package_dir);
                    return Err("C# code analysis was disabled".to_string());
                }
            }
        }
    };

    remove_temp_package(&package_dir);
    strip_temp_project_artifacts(root);

    if super::scan_project_target(root).is_some() {
        eprintln!(
            "[CsharpLsp] Unity batch generation produced project files (exit code {:?})",
            exit_code
        );
        return Ok(());
    }
    Err(batch_failure_message(exit_code, &log_tail(&log_file)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_and_batch_source_share_the_generator_chain() {
        let snippet = editor_sync_snippet();
        assert!(snippet.contains("SyncAll"));
        assert!(snippet.contains("ProjectGeneration"));
        assert!(snippet.contains("SyncVS"));
        assert!(snippet.trim_end().ends_with("print(__locusSyncReport);"));

        let source = batch_entry_point_source();
        assert!(source.contains(&format!("public static class {TEMP_ASSEMBLY_NAME}")));
        assert!(source.contains("public static void Run()"));
        assert!(source.contains("SyncAll"));
        assert!(source.contains("UnityEngine.Debug.Log"));
        assert_eq!(execute_method(), format!("{TEMP_ASSEMBLY_NAME}.Run"));
    }

    #[test]
    fn temp_package_is_written_and_removed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("Packages").join(TEMP_PACKAGE_FOLDER);
        write_temp_package(&dir).unwrap();

        let package_json = std::fs::read_to_string(dir.join("package.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&package_json).unwrap();
        assert_eq!(parsed["name"], TEMP_PACKAGE_FOLDER);

        let asmdef = std::fs::read_to_string(dir.join(format!("{TEMP_ASSEMBLY_NAME}.asmdef")))
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&asmdef).unwrap();
        assert_eq!(parsed["name"], TEMP_ASSEMBLY_NAME);
        assert_eq!(parsed["includePlatforms"][0], "Editor");

        assert!(dir.join(format!("{TEMP_ASSEMBLY_NAME}.cs")).is_file());

        remove_temp_package(&dir);
        assert!(!dir.exists());
    }

    const SLN_FIXTURE: &str = "Microsoft Visual Studio Solution File, Format Version 12.00\r\n\
# Visual Studio Version 17\r\n\
Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"Assembly-CSharp\", \"Assembly-CSharp.csproj\", \"{AAAAAAAA-0000-0000-0000-000000000001}\"\r\n\
EndProject\r\n\
Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"LocusProjectFileSyncTemp\", \"LocusProjectFileSyncTemp.csproj\", \"{BBBBBBBB-0000-0000-0000-000000000002}\"\r\n\
EndProject\r\n\
Global\r\n\
\tGlobalSection(ProjectConfigurationPlatforms) = postSolution\r\n\
\t\t{AAAAAAAA-0000-0000-0000-000000000001}.Debug|Any CPU.ActiveCfg = Debug|Any CPU\r\n\
\t\t{BBBBBBBB-0000-0000-0000-000000000002}.Debug|Any CPU.ActiveCfg = Debug|Any CPU\r\n\
\tEndGlobalSection\r\n\
EndGlobal\r\n";

    #[test]
    fn strips_temp_project_from_solution() {
        let stripped =
            strip_project_from_sln_text(SLN_FIXTURE, "LocusProjectFileSyncTemp.csproj").unwrap();
        assert!(!stripped.contains("LocusProjectFileSyncTemp"));
        assert!(!stripped.contains("{BBBBBBBB-0000-0000-0000-000000000002}"));
        assert!(stripped.contains("\"Assembly-CSharp.csproj\""));
        assert!(stripped.contains("{AAAAAAAA-0000-0000-0000-000000000001}.Debug|Any CPU.ActiveCfg"));
        assert!(stripped.contains("\r\n"));
        assert!(stripped.ends_with("\r\n"));
        // Exactly the three temp-project lines are gone.
        assert_eq!(SLN_FIXTURE.lines().count() - 3, stripped.lines().count());
    }

    #[test]
    fn solution_without_temp_project_is_untouched() {
        let sln = SLN_FIXTURE.replace("LocusProjectFileSyncTemp", "SomethingElse");
        assert!(strip_project_from_sln_text(&sln, "LocusProjectFileSyncTemp.csproj").is_none());
    }
}
