use std::sync::OnceLock;

use super::misc::truncate_utf8_prefix;
use super::{make_exec, ToolDef, ToolResult};
use crate::process_util::{async_command, augment_path_with_git, command};

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
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let command = match args.get("command").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: command".to_string(),
                            is_error: true,
                        }
                    }
                };
                let _desc = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let timeout_ms = args
                    .get("timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(120_000);
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

                let python = crate::python_runtime::resolve_effective_python(None);

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

                cmd.env("PYTHONIOENCODING", "utf-8");
                cmd.env("PYTHONUTF8", "1");
                if let Some(ref python) = python {
                    cmd.env("LOCUS_PYTHON", &python.path);
                }

                #[cfg(target_os = "windows")]
                {
                    cmd.env("GIT_CONFIG_COUNT", "1");
                    cmd.env("GIT_CONFIG_KEY_0", "core.quotePath");
                    cmd.env("GIT_CONFIG_VALUE_0", "false");
                }

                let mut path = augment_path_with_git(std::env::var_os("PATH"))
                    .or_else(|| std::env::var_os("PATH"));
                if let Some(ref python) = python {
                    path = crate::python_runtime::prepend_python_to_path(path, &python.path);
                }
                if let Some(path) = path {
                    cmd.env("PATH", path);
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
