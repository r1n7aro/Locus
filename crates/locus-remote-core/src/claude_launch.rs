//! Shared Claude launch contract. No credentials or executable paths are accepted.
use crate::claude_runtime::RuntimeSettings;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const NATIVE_BUILTIN_TOOLS: &str = "Read,Write,Edit,Bash,Glob,Grep,TaskOutput,TaskStop,AskUserQuestion,TodoWrite,TaskCreate,TaskGet,TaskList,TaskUpdate,ToolSearch,WebSearch,WebFetch";
/// Delegation builtins, offered only to conversations pinned to the
/// Claude-native multi-agent route while Multi-Agent is on.
pub const NATIVE_AGENT_TOOLS: &str = "Agent,SendMessage";
/// Inherited variables that would add Team tools or move a foreground Agent
/// to the background on a timer. Removed only for native-agent launches.
pub const NATIVE_AGENT_REMOVED_ENV: [&str; 2] =
    ["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "CLAUDE_AUTO_BACKGROUND_TASKS"];
/// Set only for native-agent launches. With fork subagents on (the default
/// in an interactive TUI) Claude forces every Agent into the background and
/// drops `run_in_background`; Locus offers no fork type. Bash keeps its own
/// background parameter, unlike with CLAUDE_CODE_DISABLE_BACKGROUND_TASKS.
pub const NATIVE_AGENT_ENV: [(&str, &str); 1] = [("CLAUDE_CODE_FORK_SUBAGENT", "0")];
pub const PROTOCOL_VERSION: u32 = 2;

/// The pinned `--tools` list. Without native agents it is exactly the
/// historical builtin list, so existing conversations keep their prefix.
pub fn native_tools(native_agents: bool) -> String {
    if native_agents {
        format!("{NATIVE_BUILTIN_TOOLS},{NATIVE_AGENT_TOOLS}")
    } else {
        NATIVE_BUILTIN_TOOLS.to_string()
    }
}
pub const AUTHORIZED_STARTUP_SECONDS: u64 = 660;
pub const SESSION_GATE_SECONDS: u64 = 45;
pub const BRIDGE_STARTUP_SECONDS: u64 = AUTHORIZED_STARTUP_SECONDS + SESSION_GATE_SECONDS + 10;
pub const MAX_SPARES: usize = 4;
pub const MAX_SPARE_STARTUPS: usize = 2;

pub fn spare_start_count(capacity: usize, spares: usize, startups: usize, retrying: bool) -> usize {
    if retrying {
        0
    } else {
        capacity
            .saturating_sub(spares)
            .min(MAX_SPARE_STARTUPS.saturating_sub(startups))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    pub protocol: u32,
    pub bridge_revision: String,
    pub features: Vec<String>,
}
impl Capabilities {
    pub fn current() -> Self {
        static REVISION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let revision = REVISION.get_or_init(|| {
            let mut hash = Sha256::new();
            hash.update(crate::claude_ssh::SSH_HOST_SOURCE);
            hash.update(crate::claude_ssh::NATIVE_MOD_SOURCE);
            format!("{:x}", hash.finalize())
        });
        Self {
            protocol: PROTOCOL_VERSION,
            bridge_revision: revision.clone(),
            features: [
                "launch-options-v1",
                "classified-errors-v1",
                "prewarm-v1",
                "owned-lifecycle-v1",
                "native-agents-v1",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }
    pub fn validate_peer(&self, peer: &Self) -> Result<(), String> {
        if peer.protocol != self.protocol
            || peer.bridge_revision != self.bridge_revision
            || self
                .features
                .iter()
                .any(|required| !peer.features.contains(required))
        {
            return Err("Claude client/server capabilities differ; update both applications before starting this conversation".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchOptions {
    pub permission_mode: PermissionMode,
    pub runtime_settings: RuntimeSettings,
    pub native_tool_search: bool,
    /// Adds the native Agent/SendMessage tools. Pinned per conversation.
    #[serde(default)]
    pub native_agents: bool,
}
impl LaunchOptions {
    pub fn apply(&self, request: &mut Value) -> Result<(), String> {
        self.runtime_settings.validate()?;
        if !request.is_object() {
            return Err("Invalid Claude launch request".into());
        }
        request["permissionMode"] = json!(self.permission_mode);
        request["runtimeSettings"] = json!(self.runtime_settings);
        request["nativeToolSearch"] = json!(self.native_tool_search);
        request["nativeTools"] = json!(native_tools(self.native_agents));
        request["nativeAgents"] = json!(self.native_agents);
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ClientMessage {
    Start {
        capabilities: Capabilities,
        launch: LaunchOptions,
        prewarm: bool,
        #[serde(rename = "clientSessionId")]
        client_session_id: String,
        resume: Option<String>,
        model: String,
        #[serde(rename = "systemPrompt")]
        system_prompt: String,
        tools: Value,
    },
    Configure {
        options: Value,
        #[serde(default)]
        uploads: Vec<Value>,
        #[serde(rename = "uploadId")]
        upload_id: Option<String>,
    },
    PasteImage {
        #[serde(rename = "inputId")]
        input_id: String,
        index: usize,
    },
    Submit,
    Cancel,
    ResumeQueued,
    Reply {
        id: String,
        value: Value,
    },
    Close,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    Transport,
    Authentication,
    Permission,
    Capability,
    Protocol,
    History,
    Capacity,
    Startup,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeError {
    pub kind: ErrorKind,
    pub stage: Option<String>,
    pub message: String,
    pub retryable: bool,
}
impl BridgeError {
    pub fn classify(error: &str) -> Self {
        let lower = error.to_ascii_lowercase();
        let retryable = crate::claude_ssh::errors::is_disconnected(error);
        let kind = if lower.contains("capabilities") || lower.contains("protocol version") {
            ErrorKind::Capability
        } else if lower.contains("authorization")
            || lower.contains("not owned")
            || lower.contains("binding changed")
        {
            ErrorKind::Permission
        } else if lower.contains("publickey")
            || lower.contains("keychain")
            || lower.contains("not authenticated")
            || lower.contains("host key")
        {
            ErrorKind::Authentication
        } else if retryable {
            ErrorKind::Transport
        } else if lower.contains("history")
            || lower.contains("native identity")
            || lower.contains("resume identity")
        {
            ErrorKind::History
        } else if lower.contains("capacity")
            || lower.contains("too many")
            || lower.contains("is busy")
        {
            ErrorKind::Capacity
        } else if lower.contains("invalid")
            || lower.contains("unsupported")
            || lower.contains("expected")
            || lower.contains("mismatch")
        {
            ErrorKind::Protocol
        } else {
            ErrorKind::Startup
        };
        let stage = [
            "SSH connection",
            "History synchronization",
            "Environment preparation",
            "TUI initialization",
        ]
        .into_iter()
        .find(|stage| error.starts_with(stage))
        .map(str::to_owned);
        Self {
            retryable: retryable && kind == ErrorKind::Transport,
            kind,
            stage,
            message: crate::claude_ssh::errors::plain_text(error)
                .chars()
                .take(8192)
                .collect(),
        }
    }
    pub fn transport(message: &str) -> Self {
        Self::classify(&crate::claude_ssh::errors::disconnected(message))
    }
    pub fn client_message(&self) -> String {
        if self.retryable {
            crate::claude_ssh::errors::disconnected(&self.message)
        } else {
            self.message.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_preserves_preferences_and_never_accepts_paths() {
        let launch = LaunchOptions {
            permission_mode: PermissionMode::BypassPermissions,
            runtime_settings: RuntimeSettings {
                idle_compaction: true,
                telemetry: true,
                auto_compact_window: Some(350_000),
            },
            native_tool_search: false,
            native_agents: false,
        };
        let mut request = json!({});
        launch.apply(&mut request).unwrap();
        assert_eq!(request["permissionMode"], "bypassPermissions");
        assert_eq!(request["runtimeSettings"]["autoCompactWindow"], 350_000);
        assert_eq!(request["nativeToolSearch"], false);
        // The builtin allowlist is pinned in every loading mode.
        assert_eq!(request["nativeTools"], NATIVE_BUILTIN_TOOLS);
        assert_eq!(request["nativeAgents"], false);
        let mut wire = serde_json::to_value(&launch).unwrap();
        wire["cliPath"] = json!("/bin/sh");
        assert!(serde_json::from_value::<LaunchOptions>(wire).is_err());
        // Older clients omit the field; they keep the historical tool list.
        let mut legacy = serde_json::to_value(&launch).unwrap();
        legacy.as_object_mut().unwrap().remove("nativeAgents");
        assert!(!serde_json::from_value::<LaunchOptions>(legacy).unwrap().native_agents);
    }
    #[test]
    fn native_agent_launches_append_only_the_delegation_tools() {
        assert_eq!(native_tools(false), NATIVE_BUILTIN_TOOLS);
        let tools = native_tools(true);
        assert!(tools.starts_with(NATIVE_BUILTIN_TOOLS));
        assert_eq!(&tools[NATIVE_BUILTIN_TOOLS.len()..], ",Agent,SendMessage");
        let mut request = json!({});
        LaunchOptions {
            permission_mode: PermissionMode::Auto,
            runtime_settings: RuntimeSettings::default(),
            native_tool_search: true,
            native_agents: true,
        }
        .apply(&mut request)
        .unwrap();
        assert_eq!(request["nativeTools"], tools);
        assert_eq!(request["nativeAgents"], true);
        assert!(!NATIVE_BUILTIN_TOOLS.split(',').any(|tool| tool == "Agent" || tool == "SendMessage"));
    }
    #[test]
    fn native_agent_conversations_never_launch_in_auto_mode() {
        assert_eq!(PermissionMode::Auto.for_launch(true), PermissionMode::BypassPermissions);
        assert_eq!(PermissionMode::BypassPermissions.for_launch(true), PermissionMode::BypassPermissions);
        // Other conversations keep the configured mode.
        assert_eq!(PermissionMode::Auto.for_launch(false), PermissionMode::Auto);
        assert_eq!(PermissionMode::BypassPermissions.for_launch(false), PermissionMode::BypassPermissions);
    }
    #[test]
    fn peers_reject_mixed_bridges_and_missing_features() {
        let caps = Capabilities::current();
        caps.validate_peer(&caps).unwrap();
        let mut old = caps.clone();
        old.protocol = 1;
        assert!(caps.validate_peer(&old).is_err());
        let mut old = caps.clone();
        old.bridge_revision = "old".into();
        assert!(caps.validate_peer(&old).is_err());
        let mut old = caps.clone();
        old.features.pop();
        assert!(caps.validate_peer(&old).is_err());
    }
    #[test]
    fn failures_keep_recovery_classification() {
        assert!(BridgeError::transport("wire lost").retryable);
        for message in [
            "Permission denied (publickey)",
            "Invalid native history",
            "Client/server capabilities differ",
            "Authorization revoked",
        ] {
            assert!(!BridgeError::classify(message).retryable);
        }
        assert_eq!(
            BridgeError::classify("TUI initialization: plugin failed")
                .stage
                .as_deref(),
            Some("TUI initialization")
        );
    }
    #[test]
    fn spare_budget_never_restarts_used_conversations() {
        assert_eq!(spare_start_count(4, 1, 0, false), 2);
        assert_eq!(spare_start_count(4, 3, 0, false), 1);
        assert_eq!(spare_start_count(4, 0, 2, false), 0);
        assert_eq!(spare_start_count(4, 0, 0, true), 0);
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Auto,
    BypassPermissions,
}
impl PermissionMode {
    /// Native-agent conversations always bypass Claude's permission layer:
    /// Locus decides every bridged call, while auto mode would route Agent
    /// starts and subagent reports through a classifier nobody can answer.
    pub fn for_launch(self, native_agents: bool) -> Self {
        if native_agents { Self::BypassPermissions } else { self }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::BypassPermissions => "bypassPermissions",
        }
    }
}
