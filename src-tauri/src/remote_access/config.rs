//! Remote access options are settings (`remoteAccess.*`, sensitive: agent
//! changes need confirmation). The installation's desktop id is identity
//! state and stays in `{persistent_config_dir}/remote_access.json`.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "remote_access.json";
/// Settings group behind every field except `desktop_id`.
const SETTINGS_GROUP: &str = "remoteAccess";
const DESKTOP_ID_FIELD: &str = "desktopId";

pub const DEFAULT_PORT: u16 = 47831;
pub const DEFAULT_TOKEN_IDLE_DAYS: u32 = 30;
const MAX_TOKEN_IDLE_DAYS: u32 = 365;
pub(crate) const MAX_HOSTNAMES: usize = 8;
pub(crate) const MAX_INTERFACES: usize = 16;

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn default_true() -> bool {
    true
}

fn default_idle_days() -> u32 {
    DEFAULT_TOKEN_IDLE_DAYS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Plain HTTP on 127.0.0.1 for tunnels that terminate TLS themselves
    /// (`tailscale serve`, UU port mapping, frp, ...).
    #[serde(default = "default_true")]
    pub loopback: bool,
    /// Adapter names (as reported by the OS) whose IPv4 addresses get an
    /// HTTPS listener with the pinned self-signed certificate.
    #[serde(default)]
    pub interfaces: Vec<String>,
    /// Hostnames fronted by a CA-trusted HTTPS proxy, e.g. the
    /// `<machine>.<tailnet>.ts.net` name of `tailscale serve`.
    #[serde(default)]
    pub hostnames: Vec<String>,
    /// Ask on the desktop before a pairing request with valid credentials
    /// receives a token.
    #[serde(default = "default_true")]
    pub require_confirmation: bool,
    /// Tokens unused for this many days stop working.
    #[serde(default = "default_idle_days")]
    pub token_idle_days: u32,
    /// Offer the bundled Locus Remote APK on the LAN while pairing (only
    /// in installers that include it).
    #[serde(default = "default_true")]
    pub app_download: bool,
    /// Stable identity of this installation, generated on first load.
    #[serde(default)]
    pub desktop_id: String,
}

impl Default for RemoteAccessSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            loopback: true,
            interfaces: Vec::new(),
            hostnames: Vec::new(),
            require_confirmation: true,
            token_idle_days: DEFAULT_TOKEN_IDLE_DAYS,
            app_download: true,
            desktop_id: String::new(),
        }
    }
}

impl RemoteAccessSettings {
    /// Clamps and canonicalizes user-editable fields. Hostnames accept pasted
    /// URLs (`https://pc.example.ts.net/`) and keep only the host.
    pub fn normalized(mut self) -> Result<Self, String> {
        if self.port < 1024 {
            return Err("端口必须在 1024–65535 之间".into());
        }
        self.token_idle_days = self.token_idle_days.clamp(1, MAX_TOKEN_IDLE_DAYS);
        let mut interfaces = Vec::new();
        for name in self.interfaces {
            let name = name.trim().to_string();
            if !name.is_empty() && !interfaces.contains(&name) {
                interfaces.push(name);
            }
        }
        if interfaces.len() > MAX_INTERFACES {
            return Err("选择的网卡过多".into());
        }
        self.interfaces = interfaces;
        let mut hostnames = Vec::new();
        for raw in self.hostnames {
            if raw.trim().is_empty() {
                continue;
            }
            let host = normalize_hostname(&raw)
                .ok_or_else(|| format!("无效的主机名：{}", raw.trim()))?;
            if !hostnames.contains(&host) {
                hostnames.push(host);
            }
        }
        if hostnames.len() > MAX_HOSTNAMES {
            return Err("登记的主机名过多".into());
        }
        self.hostnames = hostnames;
        Ok(self)
    }
}

/// `https://Pc.Example.ts.net:443/x` -> `pc.example.ts.net:443`. IP literals
/// are rejected: direct addresses are served by NIC listeners instead.
pub fn normalize_hostname(raw: &str) -> Option<String> {
    let mut value = raw.trim().to_ascii_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = value.strip_prefix(scheme) {
            value = rest.to_string();
        }
    }
    let value = value.split(['/', '?', '#']).next().unwrap_or("").to_string();
    if value.is_empty() || value.len() > 253 || value.contains('@') {
        return None;
    }
    let (host, port) = match value.rsplit_once(':') {
        Some((host, port)) => {
            let port: u16 = port.parse().ok()?;
            (host.to_string(), Some(port))
        }
        None => (value.clone(), None),
    };
    if host.parse::<std::net::IpAddr>().is_ok() || !host.contains('.') {
        return None;
    }
    let labels_ok = host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    if !labels_ok {
        return None;
    }
    Some(match port {
        Some(port) if port != 443 => format!("{host}:{port}"),
        _ => host,
    })
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(SETTINGS_FILE))
}

fn save_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

fn load_desktop_id() -> String {
    settings_path()
        .map(|path| crate::config::read_state_file(&path))
        .ok()
        .and_then(|file| file.get(DESKTOP_ID_FIELD).and_then(|id| id.as_str()).map(str::to_string))
        .unwrap_or_default()
}

fn save_desktop_id(desktop_id: &str) -> Result<(), String> {
    let path = settings_path()?;
    let _guard = save_lock().lock().unwrap_or_else(|error| error.into_inner());
    crate::config::update_state_file(&path, |file| {
        file.insert(DESKTOP_ID_FIELD.to_string(), serde_json::Value::from(desktop_id));
    })
}

/// Loads settings, generating and persisting the desktop id on first use.
pub fn load_settings() -> RemoteAccessSettings {
    let group =
        crate::settings::group::read(crate::settings::global::global(), SETTINGS_GROUP);
    let mut settings = serde_json::from_value::<RemoteAccessSettings>(group).unwrap_or_default();
    settings.token_idle_days = settings.token_idle_days.clamp(1, MAX_TOKEN_IDLE_DAYS);
    settings.desktop_id = load_desktop_id();
    if settings.desktop_id.trim().is_empty() {
        settings.desktop_id = uuid::Uuid::new_v4().simple().to_string();
        if let Err(error) = save_desktop_id(&settings.desktop_id) {
            eprintln!("[RemoteAccess] failed to persist desktop id: {error}");
        }
    }
    settings
}

pub fn save_settings(settings: &RemoteAccessSettings) -> Result<(), String> {
    let value = serde_json::to_value(settings)
        .map_err(|error| format!("无法序列化远程访问设置：{error}"))?;
    crate::settings::global::global()
        .set_values(&crate::settings::group::changes(SETTINGS_GROUP, &value))
        .map_err(|error| format!("无法保存远程访问设置：{error}"))?;
    if !settings.desktop_id.trim().is_empty() && settings.desktop_id != load_desktop_id() {
        save_desktop_id(&settings.desktop_id)?;
    }
    Ok(())
}

/// Computer name shown on the phone.
pub fn desktop_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Locus".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_with_loopback_and_confirmation() {
        let settings = RemoteAccessSettings::default();
        assert!(!settings.enabled);
        assert!(settings.loopback);
        assert!(settings.require_confirmation);
        assert_eq!(settings.port, DEFAULT_PORT);
        assert_eq!(settings.token_idle_days, DEFAULT_TOKEN_IDLE_DAYS);
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("requireConfirmation"));
        let partial: RemoteAccessSettings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(partial.loopback && partial.require_confirmation);
    }

    #[test]
    fn hostnames_accept_pasted_urls_and_reject_ip_literals() {
        assert_eq!(
            normalize_hostname("https://PC.tail1234.ts.net/").as_deref(),
            Some("pc.tail1234.ts.net")
        );
        assert_eq!(
            normalize_hostname("pc.example.net:8443").as_deref(),
            Some("pc.example.net:8443")
        );
        assert_eq!(normalize_hostname("pc.example.net:443").as_deref(), Some("pc.example.net"));
        assert!(normalize_hostname("192.168.1.3").is_none());
        assert!(normalize_hostname("localhost").is_none());
        assert!(normalize_hostname("user@host.net").is_none());
        assert!(normalize_hostname("bad_label.net").is_none());
    }

    #[test]
    fn normalization_dedupes_and_validates() {
        let settings = RemoteAccessSettings {
            interfaces: vec![" Wi-Fi ".into(), "Wi-Fi".into(), "".into()],
            hostnames: vec!["https://a.ts.net".into(), "A.ts.net".into()],
            token_idle_days: 0,
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(settings.interfaces, vec!["Wi-Fi".to_string()]);
        assert_eq!(settings.hostnames, vec!["a.ts.net".to_string()]);
        assert_eq!(settings.token_idle_days, 1);
        assert!(RemoteAccessSettings { port: 80, ..Default::default() }.normalized().is_err());
        assert!(RemoteAccessSettings { hostnames: vec!["10.0.0.1".into()], ..Default::default() }
            .normalized()
            .is_err());
    }
}
