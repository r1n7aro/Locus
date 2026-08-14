use std::path::Path;

use tree_sitter::{Node, Parser};
use tree_sitter_bash::LANGUAGE as BASH;

const MAX_WRAPPER_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DangerousCommandKind {
    ForcedRm,
    PowerShellForceDelete,
    CmdForceDelete,
    CmdRecursiveDelete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DangerousCommandMatch {
    pub(super) kind: DangerousCommandKind,
    pub(super) command: Vec<String>,
    pub(super) targets: Vec<String>,
    pub(super) unresolved_targets: bool,
}

impl DangerousCommandMatch {
    fn new(kind: DangerousCommandKind, command: &[String], targets: Vec<String>) -> Self {
        let unresolved_targets = targets.is_empty()
            || targets.iter().any(|target| {
                target.contains(['$', '*', '?', '`'])
                    || target.contains("$(")
                    || target.contains("${")
            });
        Self {
            kind,
            command: command.to_vec(),
            targets,
            unresolved_targets,
        }
    }
}

impl DangerousCommandKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ForcedRm => "forced_rm",
            Self::PowerShellForceDelete => "powershell_force_delete",
            Self::CmdForceDelete => "cmd_force_delete",
            Self::CmdRecursiveDelete => "cmd_recursive_delete",
        }
    }
}

/// Detects literal destructive commands in the script passed to Locus' bash
/// tool. This mirrors Codex' local forced-delete interception: it follows
/// literal commands through shell control flow and a bounded set of wrappers.
/// Dynamic executable names remain outside this best-effort boundary.
pub(super) fn dangerous_command_match(script: &str) -> Option<DangerousCommandMatch> {
    literal_commands(script)?
        .iter()
        .find_map(|command| match_argv(command, 0))
}

fn literal_commands(script: &str) -> Option<Vec<Vec<String>>> {
    let mut parser = Parser::new();
    parser.set_language(&BASH.into()).ok()?;
    let tree = parser.parse(script, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }

    let mut commands = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "command" {
            if let Some(command) = parse_literal_command(node, script) {
                commands.push(command);
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    Some(commands)
}

fn parse_literal_command(command: Node<'_>, source: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut found_name = false;
    let mut cursor = command.walk();
    for child in command.named_children(&mut cursor) {
        if child.kind() == "command_name" {
            words.push(parse_literal_word(child.named_child(0)?, source)?);
            found_name = true;
        } else if found_name {
            if let Some(word) = parse_literal_word(child, source) {
                words.push(word);
            }
        }
    }
    found_name.then_some(words)
}

fn parse_literal_word(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "word" | "number" if node.named_child_count() == 0 => {
            node.utf8_text(source.as_bytes()).ok().map(str::to_string)
        }
        "string" => {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|part| part.kind() != "string_content")
            {
                return None;
            }
            node.utf8_text(source.as_bytes())
                .ok()?
                .strip_prefix('"')?
                .strip_suffix('"')
                .map(str::to_string)
        }
        "raw_string" => node
            .utf8_text(source.as_bytes())
            .ok()?
            .strip_prefix('\'')?
            .strip_suffix('\'')
            .map(str::to_string),
        "concatenation" => {
            let mut value = String::new();
            let mut cursor = node.walk();
            for part in node.named_children(&mut cursor) {
                value.push_str(&parse_literal_word(part, source)?);
            }
            (!value.is_empty()).then_some(value)
        }
        _ => None,
    }
}

fn match_argv(command: &[String], depth: usize) -> Option<DangerousCommandMatch> {
    if depth > MAX_WRAPPER_DEPTH {
        return None;
    }
    let executable = command.first().and_then(|value| executable_name(value))?;
    match executable.as_str() {
        "rm" if rm_args_include_force(&command[1..]) => Some(DangerousCommandMatch::new(
            DangerousCommandKind::ForcedRm,
            command,
            rm_targets(&command[1..]),
        )),
        "sudo" => match_argv(&command[1..], depth + 1),
        "env" => match_env(command, depth + 1),
        "trap" => match_trap(command, depth + 1),
        "sh" | "bash" | "zsh" => match_nested_shell(command, depth + 1),
        "pwsh" | "powershell" => match_powershell(command),
        "cmd" => match_cmd(command),
        _ => None,
    }
}

fn executable_name(raw: &str) -> Option<String> {
    let normalized = raw.replace('\\', "/");
    let name = Path::new(&normalized)
        .file_name()?
        .to_str()?
        .to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(value) = name.strip_suffix(suffix) {
            return Some(value.to_string());
        }
    }
    Some(name)
}

fn rm_args_include_force(args: &[String]) -> bool {
    args.iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| {
            arg == "--force"
                || arg
                    .strip_prefix('-')
                    .is_some_and(|flags| !flags.starts_with('-') && flags.contains('f'))
        })
}

fn rm_targets(args: &[String]) -> Vec<String> {
    let mut after_options = false;
    args.iter()
        .filter_map(|argument| {
            if !after_options && argument == "--" {
                after_options = true;
                return None;
            }
            if !after_options && argument.starts_with('-') {
                return None;
            }
            Some(argument.clone())
        })
        .collect()
}

fn match_env(command: &[String], depth: usize) -> Option<DangerousCommandMatch> {
    let mut index = 1;
    while let Some(argument) = command.get(index) {
        if argument == "--" {
            index += 1;
            break;
        }
        if matches!(argument.as_str(), "-i" | "--ignore-environment")
            || argument
                .split_once('=')
                .is_some_and(|(name, _)| !name.is_empty() && !name.starts_with('-'))
        {
            index += 1;
            continue;
        }
        break;
    }
    match_argv(&command[index..], depth)
}

fn match_trap(command: &[String], depth: usize) -> Option<DangerousCommandMatch> {
    let mut index = 1;
    if command.get(index).is_some_and(|value| value == "--") {
        index += 1;
    }
    let action = command.get(index).filter(|value| !value.starts_with('-'))?;
    match_script(action, depth)
}

fn match_nested_shell(command: &[String], depth: usize) -> Option<DangerousCommandMatch> {
    let script_index = command
        .iter()
        .position(|value| matches!(value.as_str(), "-c" | "-lc"))?
        + 1;
    match_script(command.get(script_index)?, depth)
}

fn match_script(script: &str, depth: usize) -> Option<DangerousCommandMatch> {
    if depth > MAX_WRAPPER_DEPTH {
        return None;
    }
    literal_commands(script)?
        .iter()
        .find_map(|command| match_argv(command, depth))
}

fn match_powershell(command: &[String]) -> Option<DangerousCommandMatch> {
    let script_index = command.iter().position(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "-command" | "-c" | "/c"
        )
    })? + 1;
    let script = command.get(script_index..)?.join(" ");
    script
        .split([';', '|', '&', '\n', '\r'])
        .find_map(|segment| {
            let words = split_words(segment);
            let has_delete = words.iter().any(|word| {
                matches!(
                    word.as_str(),
                    "remove-item" | "ri" | "rm" | "del" | "erase" | "rd" | "rmdir"
                )
            });
            let has_force = words
                .iter()
                .any(|word| word == "-force" || word.starts_with("-force:"));
            if !has_delete || !has_force {
                return None;
            }
            let targets = words
                .iter()
                .filter(|word| {
                    !word.starts_with('-')
                        && !matches!(
                            word.as_str(),
                            "remove-item" | "ri" | "rm" | "del" | "erase" | "rd" | "rmdir"
                        )
                })
                .cloned()
                .collect();
            Some(DangerousCommandMatch::new(
                DangerousCommandKind::PowerShellForceDelete,
                &words,
                targets,
            ))
        })
}

fn match_cmd(command: &[String]) -> Option<DangerousCommandMatch> {
    let body_index = command
        .iter()
        .position(|value| matches!(value.to_ascii_lowercase().as_str(), "/c" | "/r" | "-c"))?
        + 1;
    let body = command.get(body_index..)?.join(" ");
    body.split(['&', '|', '\n', '\r']).find_map(|segment| {
        let words = split_words(segment);
        let executable = words.first()?.as_str();
        if matches!(executable, "del" | "erase") && words.iter().any(|word| word == "/f") {
            let targets = words
                .iter()
                .skip(1)
                .filter(|word| !word.starts_with('/'))
                .cloned()
                .collect();
            return Some(DangerousCommandMatch::new(
                DangerousCommandKind::CmdForceDelete,
                &words,
                targets,
            ));
        }
        if matches!(executable, "rd" | "rmdir")
            && words.iter().any(|word| word == "/s")
            && words.iter().any(|word| word == "/q")
        {
            let targets = words
                .iter()
                .skip(1)
                .filter(|word| !word.starts_with('/'))
                .cloned()
                .collect();
            return Some(DangerousCommandMatch::new(
                DangerousCommandKind::CmdRecursiveDelete,
                &words,
                targets,
            ));
        }
        None
    })
}

fn split_words(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| {
                matches!(ch, '\'' | '"' | '(' | ')' | '{' | '}' | '[' | ']' | ',')
            })
            .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_forced_rm_through_shell_syntax_and_wrappers() {
        for command in [
            "rm -rf /tmp/example",
            "sudo rm -r -f /tmp/example",
            "env FOO=bar rm --force /tmp/example",
            "for path in a b; do rm -fr \"$path\"; done",
            "echo $(rm -rf /tmp/example)",
            "trap 'rm -rf /tmp/example' EXIT",
            "bash -lc 'rm -rf /tmp/example'",
        ] {
            assert_eq!(
                dangerous_command_match(command).map(|matched| matched.kind),
                Some(DangerousCommandKind::ForcedRm),
                "command={command}"
            );
        }
    }

    #[test]
    fn keeps_non_forced_and_dynamic_rm_outside_the_match() {
        for command in [
            "rm -r /tmp/example",
            "rm -- -f",
            "cmd=rm; $cmd -rf /tmp/example",
        ] {
            assert_eq!(dangerous_command_match(command), None, "command={command}");
        }
    }

    #[test]
    fn detects_windows_force_delete_invocations() {
        assert_eq!(
            dangerous_command_match("pwsh -Command 'Remove-Item -Recurse -Force C:/Temp/example'")
                .map(|matched| matched.kind),
            Some(DangerousCommandKind::PowerShellForceDelete)
        );
        assert_eq!(
            dangerous_command_match("cmd.exe /c del /f C:\\\\Temp\\\\example.txt")
                .map(|matched| matched.kind),
            Some(DangerousCommandKind::CmdForceDelete)
        );
        assert_eq!(
            dangerous_command_match("cmd /c rmdir /s /q C:\\\\Temp\\\\example")
                .map(|matched| matched.kind),
            Some(DangerousCommandKind::CmdRecursiveDelete)
        );
    }

    #[test]
    fn exposes_literal_targets_and_marks_dynamic_targets_unresolved() {
        let matched = dangerous_command_match("rm -rf -- ./Library ./Temp").expect("match");
        assert_eq!(matched.targets, vec!["./Library", "./Temp"]);
        assert!(!matched.unresolved_targets);

        let dynamic = dangerous_command_match("for path in a b; do rm -rf \"$path\"; done")
            .expect("dynamic match");
        assert!(dynamic.targets.is_empty());
        assert!(dynamic.unresolved_targets);
    }
}
