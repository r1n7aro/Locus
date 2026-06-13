use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::misc::truncate_utf8_prefix;
use super::{make_exec, ToolDef, ToolResult};
use crate::process_util::{
    async_command, augment_path_with_git, augment_path_with_github_cli, command,
};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
// Interactive commands wait for a human to finish typing in a terminal window,
// so the default budget is much larger than the regular one.
const INTERACTIVE_DEFAULT_TIMEOUT_MS: u64 = 600_000;
const INTERACTIVE_POLL_INTERVAL_MS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Sh,  // sh / Git Bash
    Cmd, // cmd.exe
}

pub fn detect_shell() -> ShellKind {
    static SHELL: OnceLock<ShellKind> = OnceLock::new();
    *SHELL.get_or_init(|| {
        if cfg!(target_os = "windows") {
            let mut probe = command("sh");
            probe
                .arg("-c")
                .arg("echo ok")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Some(path) = augment_path_with_git(std::env::var_os("PATH")) {
                probe.env("PATH", path);
            }
            let ok = probe.status().map(|s| s.success()).unwrap_or(false);
            if ok {
                ShellKind::Sh
            } else {
                ShellKind::Cmd
            }
        } else {
            ShellKind::Sh
        }
    })
}

pub fn shell_display_name() -> &'static str {
    match detect_shell() {
        ShellKind::Sh => {
            if cfg!(target_os = "windows") {
                "sh (Git Bash)"
            } else {
                "sh"
            }
        }
        ShellKind::Cmd => "cmd.exe",
    }
}

pub(super) fn bash() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::BASH);
    ToolDef {
        name: "bash".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Arbitrary shell commands can touch anything in the workspace.
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let command = match args.get("command").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: command".to_string(),
                            is_error: true,
                        };
                    }
                };
                let _desc = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let interactive = args
                    .get("interactive")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let timeout_ms =
                    args.get("timeout")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(if interactive {
                            INTERACTIVE_DEFAULT_TIMEOUT_MS
                        } else {
                            DEFAULT_TIMEOUT_MS
                        });
                let workdir = args
                    .get("workdir")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                if workdir.is_none() {
                    return ToolResult {
                        output: "Missing required parameter: workdir".to_string(),
                        is_error: true,
                    };
                }

                let python =
                    crate::python_runtime::resolve_effective_python(ctx.app_handle.as_ref());
                if let Some(ref python) = python {
                    if let Err(error) =
                        crate::python_runtime::ensure_runtime_package_environment(python)
                    {
                        return ToolResult {
                            output: error,
                            is_error: true,
                        };
                    }
                }

                let sh_command = || {
                    if let Some(ref python) = python {
                        format!(
                            "{}{}",
                            crate::python_runtime::sh_python_function_prefix(&python.path),
                            command
                        )
                    } else {
                        command.clone()
                    }
                };

                let envs = collect_shell_env(python.as_ref());

                if interactive {
                    return run_interactive_command(
                        &command,
                        &sh_command(),
                        workdir.as_deref().unwrap_or_default(),
                        &envs,
                        timeout_ms,
                    )
                    .await;
                }

                let mut cmd = if cfg!(target_os = "windows") {
                    if detect_shell() == ShellKind::Sh {
                        let mut c = async_command("sh");
                        c.arg("-c").arg(sh_command());
                        c
                    } else {
                        let wrapped = format!("chcp 65001 >nul && {}", command);
                        let mut c = async_command("cmd");
                        c.arg("/S").arg("/C").arg(&wrapped);
                        c
                    }
                } else {
                    let mut c = async_command("sh");
                    c.arg("-c").arg(sh_command());
                    c
                };
                cmd.stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());

                for (key, value) in &envs {
                    cmd.env(key, value);
                }

                if let Some(ref dir) = workdir {
                    cmd.current_dir(dir);
                }

                let result = tokio::time::timeout(
                    std::time::Duration::from_millis(timeout_ms),
                    cmd.output(),
                )
                .await;

                match result {
                    Ok(Ok(output)) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let stderr = String::from_utf8_lossy(&output.stderr);

                        let mut out = String::new();
                        out.push_str(&stdout);
                        out.push_str(&stderr);

                        if out.len() > 50_000 {
                            let total_bytes = out.len();
                            let truncated = truncate_utf8_prefix(&out, 50_000);
                            out = format!(
                                "{}...\n\n(output truncated, {} bytes total)",
                                truncated, total_bytes
                            );
                        }

                        if out.is_empty() {
                            out = "(no output)".to_string();
                        }

                        let exit_code = output.status.code().unwrap_or(-1);
                        ToolResult {
                            output: format!("Exit code: {}\n{}", exit_code, out),
                            is_error: exit_code != 0,
                        }
                    }
                    Ok(Err(e)) => ToolResult {
                        output: format!("Failed to execute command: {}", e),
                        is_error: true,
                    },
                    Err(_) => ToolResult {
                        output: format!("Command timed out after {}ms: {}", timeout_ms, command),
                        is_error: true,
                    },
                }
            })
        }),
    }
}

fn collect_shell_env(
    python: Option<&crate::python_runtime::ResolvedPythonRuntime>,
) -> Vec<(String, OsString)> {
    let mut envs: Vec<(String, OsString)> = Vec::new();

    // Fill in system (registry) variables missing from the process snapshot,
    // e.g. JAVA_HOME registered by a tool installed after Locus started.
    // Gap-fill only: session/launcher values are never overridden, and the
    // Locus-managed keys below win because later env() calls take precedence.
    #[cfg(target_os = "windows")]
    for (key, value) in crate::process_util::read_registry_env_entries() {
        if std::env::var_os(&key).is_none() {
            envs.push((key, value.into()));
        }
    }

    envs.push(("PYTHONIOENCODING".to_string(), OsString::from("utf-8")));
    envs.push(("PYTHONUTF8".to_string(), OsString::from("1")));
    if let Some(python) = python {
        envs.push((
            "LOCUS_PYTHON".to_string(),
            python.path.clone().into_os_string(),
        ));
        if let Some(ref home) = python.home {
            envs.push(("PYTHONHOME".to_string(), home.clone().into_os_string()));
        }
        if matches!(
            &python.source,
            crate::python_runtime::PythonRuntimeSource::Managed
        ) {
            envs.push(("PYTHONNOUSERSITE".to_string(), OsString::from("1")));
            envs.push((
                "PIP_DISABLE_PIP_VERSION_CHECK".to_string(),
                OsString::from("1"),
            ));
            envs.push((
                "PIP_NO_WARN_SCRIPT_LOCATION".to_string(),
                OsString::from("1"),
            ));
            if let Some(ref package_dir) = python.package_dir {
                envs.push((
                    "PIP_TARGET".to_string(),
                    package_dir.clone().into_os_string(),
                ));
                if let Some(python_path) = crate::python_runtime::managed_python_path_env(
                    std::env::var_os("PYTHONPATH"),
                    python,
                ) {
                    envs.push(("PYTHONPATH".to_string(), python_path));
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        envs.push(("GIT_CONFIG_COUNT".to_string(), OsString::from("1")));
        envs.push((
            "GIT_CONFIG_KEY_0".to_string(),
            OsString::from("core.quotePath"),
        ));
        envs.push(("GIT_CONFIG_VALUE_0".to_string(), OsString::from("false")));
    }

    // Merge in registry PATH entries first (appended), then prepend the
    // Locus-managed runtimes so they take precedence over machine installs.
    let mut path = crate::process_util::augment_path_with_registry_paths(std::env::var_os("PATH"))
        .or_else(|| std::env::var_os("PATH"));
    path = augment_path_with_git(path.clone()).or(path);
    path = augment_path_with_github_cli(path.clone()).or(path);
    if let Some(python) = python {
        path = crate::python_runtime::prepend_python_to_path(path, &python.path);
    }
    if let Some(path) = path {
        envs.push(("PATH".to_string(), path));
    }

    envs
}

async fn run_interactive_command(
    raw_command: &str,
    sh_command: &str,
    workdir: &str,
    envs: &[(String, OsString)],
    timeout_ms: u64,
) -> ToolResult {
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let temp_dir = std::env::temp_dir();
    let marker_path = temp_dir.join(format!("locus-interactive-{}.exit", run_id));

    let use_cmd_script = cfg!(target_os = "windows") && detect_shell() == ShellKind::Cmd;
    let (script_path, script_content) = if use_cmd_script {
        (
            temp_dir.join(format!("locus-interactive-{}.cmd", run_id)),
            build_interactive_cmd_script(raw_command, workdir, &marker_path),
        )
    } else {
        // On Windows the terminal inherits our env through the launcher, and
        // MSYS converts the Windows-format PATH to POSIX form on shell
        // startup; exporting the Windows-format values inside the script
        // would clobber that conversion and break all command lookup (127).
        // macOS/Linux terminal apps do not inherit our env, so the script
        // must export it there.
        let script_envs: &[(String, OsString)] = if cfg!(target_os = "windows") {
            &[]
        } else {
            envs
        };
        // Runs before the user's command and never affects its exit status;
        // only conhost needs it, other platforms handle ANSI natively.
        let sh_command_with_vt;
        let sh_command = if cfg!(target_os = "windows") {
            sh_command_with_vt = format!("{}\n{}", vt_enable_sh_line(), sh_command);
            sh_command_with_vt.as_str()
        } else {
            sh_command
        };
        (
            temp_dir.join(format!("locus-interactive-{}.sh", run_id)),
            build_interactive_sh_script(sh_command, workdir, script_envs, &marker_path),
        )
    };
    if let Err(error) = std::fs::write(&script_path, &script_content) {
        return ToolResult {
            output: format!(
                "Failed to prepare the interactive command script: {}",
                error
            ),
            is_error: true,
        };
    }

    let (child, launcher_path) = match spawn_interactive_terminal(
        &script_path,
        use_cmd_script,
        workdir,
        envs,
        &temp_dir,
        &run_id,
    ) {
        Ok(spawned) => spawned,
        Err(message) => {
            let _ = std::fs::remove_file(&script_path);
            return ToolResult {
                output: message,
                is_error: true,
            };
        }
    };

    let result = wait_for_interactive_exit(&marker_path, timeout_ms, child, raw_command).await;

    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&marker_path);
    if let Some(ref launcher_path) = launcher_path {
        let _ = std::fs::remove_file(launcher_path);
    }
    result
}

async fn wait_for_interactive_exit(
    marker_path: &Path,
    timeout_ms: u64,
    mut child: Option<tokio::process::Child>,
    command: &str,
) -> ToolResult {
    let started = std::time::Instant::now();
    let mut terminal_gone_at: Option<std::time::Instant> = None;
    loop {
        if let Some(exit_code) = read_interactive_exit_code(marker_path) {
            let mut output = format!(
                "Interactive command finished with exit code {}.\nOutput was shown only in the terminal window and was not captured; verify the outcome with a non-interactive follow-up command if needed.",
                exit_code
            );
            if exit_code != 0 {
                output.push('\n');
                output.push_str(interactive_failure_hint(exit_code));
            }
            return ToolResult {
                output,
                is_error: exit_code != 0,
            };
        }

        if let Some(ref mut child) = child {
            if matches!(child.try_wait(), Ok(Some(_))) && terminal_gone_at.is_none() {
                terminal_gone_at = Some(std::time::Instant::now());
            }
        }
        // On a normal finish the marker is written before the window closes,
        // so give it a grace period after the terminal process ends.
        if let Some(gone_at) = terminal_gone_at {
            if gone_at.elapsed() >= std::time::Duration::from_millis(1_000) {
                return ToolResult {
                    output: "The interactive terminal window was closed before the command finished; no exit status was recorded.".to_string(),
                    is_error: true,
                };
            }
        }

        if started.elapsed() >= std::time::Duration::from_millis(timeout_ms) {
            if let Some(mut child) = child.take() {
                let _ = child.start_kill();
            }
            return ToolResult {
                output: format!(
                    "Interactive command timed out after {}ms: {}\nThe terminal window may still be open; the user should close it manually.",
                    timeout_ms, command
                ),
                is_error: true,
            };
        }

        tokio::time::sleep(std::time::Duration::from_millis(
            INTERACTIVE_POLL_INTERVAL_MS,
        ))
        .await;
    }
}

// The terminal's output cannot be captured without a PTY, so on failure the
// model needs a path to the error text: reproducing the failure in capture
// mode. Startup errors fail identically in both modes.
fn interactive_failure_hint(exit_code: i32) -> &'static str {
    match exit_code {
        127 => "Exit code 127 means a program in the command was not found on PATH (or the working directory could not be entered) before any interaction happened. Rerun the same command with interactive=false to capture the exact error output, fix the command, then retry interactively.",
        126 => "Exit code 126 means a program in the command was found but could not be executed. Rerun the same command with interactive=false to capture the exact error output.",
        _ => "If the failure happened before any user input was needed (e.g. a startup error), rerun the same command with interactive=false to capture its error output, fix it, and retry interactively.",
    }
}

fn read_interactive_exit_code(marker_path: &Path) -> Option<i32> {
    let content = std::fs::read_to_string(marker_path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        // The marker exists but the status has not been flushed yet.
        return None;
    }
    Some(trimmed.parse::<i32>().unwrap_or(-1))
}

#[cfg(target_os = "windows")]
fn spawn_interactive_terminal(
    script_path: &Path,
    use_cmd_script: bool,
    workdir: &str,
    envs: &[(String, OsString)],
    temp_dir: &Path,
    run_id: &str,
) -> Result<(Option<tokio::process::Child>, Option<PathBuf>), String> {
    // `start` treats its first quoted argument as the window title, so the
    // program path can be quoted safely. Routing `start` through a launcher
    // script avoids the quoting pitfalls of passing it via `cmd /C` arguments.
    let launch_line = if use_cmd_script {
        format!(
            "@start \"Locus Interactive\" /WAIT cmd /C \"{}\"\r\n",
            script_path.display()
        )
    } else {
        let sh_path = find_sh_for_interactive(envs).ok_or_else(|| {
            "Failed to locate sh (Git Bash) for the interactive terminal.".to_string()
        })?;
        format!(
            "@start \"Locus Interactive\" /WAIT \"{}\" \"{}\"\r\n",
            sh_path.display(),
            script_path.display()
        )
    };
    let launcher_path = temp_dir.join(format!("locus-interactive-{}-launch.cmd", run_id));
    if let Err(error) = std::fs::write(&launcher_path, launch_line) {
        return Err(format!(
            "Failed to prepare the interactive terminal launcher: {}",
            error
        ));
    }

    let mut cmd = async_command("cmd");
    cmd.arg("/C").arg(&launcher_path);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.current_dir(workdir);
    // The launcher lives for as long as the terminal window; killing it on
    // cancellation stops the wait without leaving the helper process behind.
    cmd.kill_on_drop(true);
    match cmd.spawn() {
        Ok(child) => Ok((Some(child), Some(launcher_path))),
        Err(error) => {
            let _ = std::fs::remove_file(&launcher_path);
            Err(format!(
                "Failed to open the interactive terminal: {}",
                error
            ))
        }
    }
}

#[cfg(target_os = "macos")]
fn spawn_interactive_terminal(
    script_path: &Path,
    _use_cmd_script: bool,
    _workdir: &str,
    _envs: &[(String, OsString)],
    _temp_dir: &Path,
    _run_id: &str,
) -> Result<(Option<tokio::process::Child>, Option<PathBuf>), String> {
    // Terminal.app does not inherit our environment; the script exports it.
    let invocation = format!("/bin/sh '{}'", script_path.display());
    let escaped = invocation.replace('\\', "\\\\").replace('"', "\\\"");
    let mut cmd = async_command("osascript");
    cmd.arg("-e")
        .arg("tell application \"Terminal\" to activate")
        .arg("-e")
        .arg(format!(
            "tell application \"Terminal\" to do script \"{}\"",
            escaped
        ));
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        // osascript returns as soon as the Terminal window is created, so the
        // child is not useful for tracking the command itself.
        Ok(_) => Ok((None, None)),
        Err(error) => Err(format!("Failed to open Terminal: {}", error)),
    }
}

#[cfg(target_os = "linux")]
fn spawn_interactive_terminal(
    script_path: &Path,
    _use_cmd_script: bool,
    _workdir: &str,
    _envs: &[(String, OsString)],
    _temp_dir: &Path,
    _run_id: &str,
) -> Result<(Option<tokio::process::Child>, Option<PathBuf>), String> {
    let script = script_path.display().to_string();
    let attempts: [(&str, &[&str]); 4] = [
        ("x-terminal-emulator", &["-e", "sh"]),
        ("gnome-terminal", &["--", "sh"]),
        ("konsole", &["-e", "sh"]),
        ("xterm", &["-e", "sh"]),
    ];
    for (program, args) in attempts {
        let mut cmd = async_command(program);
        cmd.args(args).arg(&script);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if cmd.spawn().is_ok() {
            // Terminal emulators may daemonize immediately; completion is
            // tracked through the exit marker instead of the child process.
            return Ok((None, None));
        }
    }
    Err(
        "No terminal emulator found for the interactive command (tried x-terminal-emulator, gnome-terminal, konsole, xterm)."
            .to_string(),
    )
}

#[cfg(target_os = "windows")]
fn find_sh_for_interactive(envs: &[(String, OsString)]) -> Option<PathBuf> {
    let path_var = envs
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var_os("PATH"))?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("sh.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// Classic conhost windows (opened via `start`) do not process ANSI escapes
// by default, so TUI prompts re-render as appended duplicates instead of
// redrawing in place. VT processing is a property of the console screen
// buffer: enabling it once before the user's command makes every later write
// in that window render correctly. The PowerShell snippet is passed as
// -EncodedCommand (base64 of UTF-16LE) to stay immune to sh/cmd quoting.
fn vt_enable_encoded_command() -> &'static str {
    static ENCODED: OnceLock<String> = OnceLock::new();
    ENCODED.get_or_init(|| {
        use base64::Engine;
        const SOURCE: &str = concat!(
            "try{$sig='[DllImport(\"kernel32.dll\")]public static extern IntPtr GetStdHandle(int n);",
            "[DllImport(\"kernel32.dll\")]public static extern bool GetConsoleMode(IntPtr h,out uint m);",
            "[DllImport(\"kernel32.dll\")]public static extern bool SetConsoleMode(IntPtr h,uint m);';",
            "$k=Add-Type -MemberDefinition $sig -Name Vt -Namespace LocusConsole -PassThru;",
            "foreach($n in @(-11,-12)){$h=$k::GetStdHandle($n);$m=0;",
            "if($k::GetConsoleMode($h,[ref]$m)){[void]$k::SetConsoleMode($h,$m -bor 4)}}}catch{}",
        );
        let utf16: Vec<u8> = SOURCE
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        base64::engine::general_purpose::STANDARD.encode(utf16)
    })
}

fn vt_enable_sh_line() -> String {
    format!(
        "powershell.exe -NoProfile -NonInteractive -EncodedCommand {} >/dev/null 2>&1 || true",
        vt_enable_encoded_command()
    )
}

fn sh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn build_interactive_sh_script(
    command: &str,
    workdir: &str,
    envs: &[(String, OsString)],
    marker_path: &Path,
) -> String {
    let marker = sh_single_quote(&marker_path.display().to_string());
    let mut script = String::from("#!/bin/sh\n");
    for (key, value) in envs {
        script.push_str("export ");
        script.push_str(key);
        script.push('=');
        script.push_str(&sh_single_quote(&value.to_string_lossy()));
        script.push('\n');
    }
    script.push_str(&format!(
        "cd {} || {{ echo \"[Locus] Failed to enter the working directory.\"; echo 127 > {}; read __locus_unused; exit 127; }}\n",
        sh_single_quote(workdir),
        marker
    ));
    script.push_str(command);
    script.push('\n');
    script.push_str("__locus_status=$?\n");
    script.push_str(&format!("echo \"$__locus_status\" > {}\n", marker));
    script.push_str("echo\n");
    script.push_str(
        "echo \"[Locus] Command finished with exit code $__locus_status. You can close this window and return to Locus.\"\n",
    );
    script.push_str("read __locus_unused\n");
    script.push_str("exit \"$__locus_status\"\n");
    script
}

fn build_interactive_cmd_script(command: &str, workdir: &str, marker_path: &Path) -> String {
    let marker = marker_path.display();
    format!(
        concat!(
            "@echo off\r\n",
            "chcp 65001 >nul\r\n",
            "powershell.exe -NoProfile -NonInteractive -EncodedCommand {vt} >nul 2>&1\r\n",
            "cd /d \"{workdir}\"\r\n",
            "if errorlevel 1 (\r\n",
            "  echo [Locus] Failed to enter the working directory.\r\n",
            "  >\"{marker}\" echo 127\r\n",
            "  pause >nul\r\n",
            "  exit /b 127\r\n",
            ")\r\n",
            "{command}\r\n",
            "set \"__LOCUS_STATUS=%ERRORLEVEL%\"\r\n",
            ">\"{marker}\" echo %__LOCUS_STATUS%\r\n",
            "echo.\r\n",
            "echo [Locus] Command finished with exit code %__LOCUS_STATUS%. You can close this window and return to Locus.\r\n",
            "pause >nul\r\n",
            "exit /b %__LOCUS_STATUS%\r\n",
        ),
        workdir = workdir,
        marker = marker,
        command = command,
        vt = vt_enable_encoded_command(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_single_quote_escapes_embedded_quotes() {
        assert_eq!(sh_single_quote("plain"), "'plain'");
        assert_eq!(sh_single_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn interactive_sh_script_exports_env_and_records_exit_code() {
        let envs = vec![("PATH".to_string(), OsString::from("/usr/bin"))];
        let script = build_interactive_sh_script(
            "gh auth login",
            "/work dir",
            &envs,
            Path::new("/tmp/marker.exit"),
        );
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("export PATH='/usr/bin'\n"));
        assert!(script.contains("cd '/work dir' ||"));
        assert!(script.contains("gh auth login\n"));
        assert!(script.contains("echo \"$__locus_status\" > '/tmp/marker.exit'\n"));
        assert!(script.contains("read __locus_unused\n"));
    }

    #[test]
    fn interactive_failure_hint_points_to_capture_mode() {
        assert!(interactive_failure_hint(127).contains("interactive=false"));
        assert!(interactive_failure_hint(126).contains("interactive=false"));
        assert!(interactive_failure_hint(1).contains("interactive=false"));
    }

    #[test]
    fn interactive_cmd_script_redirects_before_echoing_status() {
        let script = build_interactive_cmd_script(
            "gh auth login",
            "C:\\work",
            Path::new("C:\\temp\\marker.exit"),
        );
        assert!(script.contains("cd /d \"C:\\work\"\r\n"));
        assert!(script.contains("gh auth login\r\n"));
        // The redirection must come first: a digit at the end of the echoed
        // text would otherwise be parsed as a file-descriptor redirection.
        assert!(script.contains(">\"C:\\temp\\marker.exit\" echo %__LOCUS_STATUS%\r\n"));
        assert!(script.contains("pause >nul\r\n"));
        assert!(script.contains("-EncodedCommand"));
    }

    #[test]
    fn vt_enable_line_is_quoting_safe_and_never_fails() {
        let line = vt_enable_sh_line();
        assert!(line.contains("-EncodedCommand"));
        assert!(line.ends_with("|| true"));
        // base64 payload must not need any shell quoting
        let encoded = vt_enable_encoded_command();
        assert!(!encoded.is_empty());
        assert!(encoded
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='));
    }
}
