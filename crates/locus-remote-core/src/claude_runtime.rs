//! Per-process preferences shared by local terminals, probes and SSH hosts.
use std::collections::HashMap;
use std::ffi::OsString;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSettings {
    #[serde(default)]
    pub idle_compaction: bool,
    #[serde(default)]
    pub telemetry: bool,
    /// Native CLI auto-compact window in tokens; None leaves the CLI's policy alone.
    #[serde(default)]
    pub auto_compact_window: Option<u32>,
}

impl RuntimeSettings {
    pub fn validate(self) -> Result<(), String> {
        if self
            .auto_compact_window
            .is_some_and(|tokens| !(100_000..=1_000_000).contains(&tokens))
        {
            return Err(
                "Claude Code auto-compact window must be between 100000 and 1000000 tokens".into(),
            );
        }
        Ok(())
    }

    pub fn cli_settings(self, bypass_permissions: bool) -> serde_json::Value {
        // Locus never uses cross-session messaging. Refuse peers from other
        // Claude sessions; macOS SSH hosts share the user's own profile.
        let mut settings = serde_json::json!({
            "idleCompaction": self.idle_compaction,
            "crossSessionInbound": "refuse",
        });
        if bypass_permissions {
            settings["skipDangerousModePermissionPrompt"] = serde_json::json!(true);
        }
        settings
    }

    pub fn apply_env(self, env: &mut HashMap<OsString, OsString>) {
        // The native override takes precedence over saved per-model windows.
        // Do not change the host's own configuration when no override is set.
        if let Some(tokens) = self.auto_compact_window {
            env.insert(
                "CLAUDE_CODE_AUTO_COMPACT_WINDOW".into(),
                tokens.to_string().into(),
            );
        }
        // The unprefixed name is not recognized by supported Claude builds.
        env.remove(std::ffi::OsStr::new("DISABLE_NONESSENTIAL_TRAFFIC"));
        for key in [
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "DISABLE_TELEMETRY",
            "DISABLE_ERROR_REPORTING",
        ] {
            if self.telemetry {
                // These variables test presence: "0" still disables traffic.
                env.remove(std::ffi::OsStr::new(key));
            } else {
                env.insert(key.into(), "1".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_disable_idle_compaction_without_disabling_normal_compaction() {
        let settings = RuntimeSettings::default();
        assert_eq!(
            settings.cli_settings(false),
            serde_json::json!({"idleCompaction": false, "crossSessionInbound": "refuse"})
        );
        assert_eq!(
            settings.cli_settings(true),
            serde_json::json!({"idleCompaction": false, "crossSessionInbound": "refuse", "skipDangerousModePermissionPrompt": true})
        );
    }

    #[test]
    fn native_auto_compact_window_validates_and_roundtrips() {
        let legacy: RuntimeSettings = serde_json::from_str(r#"{"idleCompaction":true}"#).unwrap();
        assert_eq!(legacy.auto_compact_window, None);
        for tokens in [100_000, 350_000, 1_000_000] {
            let settings = RuntimeSettings {
                auto_compact_window: Some(tokens),
                ..Default::default()
            };
            assert!(settings.validate().is_ok());
            let wire = serde_json::to_value(settings).unwrap();
            assert_eq!(wire["autoCompactWindow"], tokens);
            assert_eq!(
                serde_json::from_value::<RuntimeSettings>(wire).unwrap(),
                settings
            );
        }
        for tokens in [0, 99_999, 1_000_001, u32::MAX] {
            assert!(RuntimeSettings {
                auto_compact_window: Some(tokens),
                ..Default::default()
            }
            .validate()
            .is_err());
        }
        assert!(RuntimeSettings::default().validate().is_ok());
    }

    #[test]
    fn native_auto_compact_window_overrides_only_when_configured() {
        let key = std::ffi::OsStr::new("CLAUDE_CODE_AUTO_COMPACT_WINDOW");
        let mut env = HashMap::new();
        RuntimeSettings::default().apply_env(&mut env);
        assert!(!env.contains_key(key));
        env.insert(key.into(), "500000".into());
        RuntimeSettings::default().apply_env(&mut env);
        assert_eq!(env.get(key), Some(&OsString::from("500000")));
        RuntimeSettings {
            auto_compact_window: Some(350_000),
            ..Default::default()
        }
        .apply_env(&mut env);
        assert_eq!(env.get(key), Some(&OsString::from("350000")));
    }

    #[test]
    fn telemetry_preferences_use_supported_names_and_preserve_provider_and_gzip() {
        let mut env = HashMap::from([
            ("DISABLE_NONESSENTIAL_TRAFFIC".into(), "1".into()),
            ("ANTHROPIC_BASE_URL".into(), "https://example.test".into()),
            ("CLAUDE_CODE_GZIP_REQUEST_BODIES".into(), "1".into()),
        ]);
        RuntimeSettings::default().apply_env(&mut env);
        assert!(!env.contains_key(std::ffi::OsStr::new("DISABLE_NONESSENTIAL_TRAFFIC")));
        for key in [
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "DISABLE_TELEMETRY",
            "DISABLE_ERROR_REPORTING",
        ] {
            assert_eq!(
                env.get(std::ffi::OsStr::new(key)),
                Some(&OsString::from("1"))
            );
        }
        RuntimeSettings {
            telemetry: true,
            idle_compaction: true,
            ..Default::default()
        }
        .apply_env(&mut env);
        assert_eq!(env.len(), 2);
        assert_eq!(
            env.get(std::ffi::OsStr::new("CLAUDE_CODE_GZIP_REQUEST_BODIES")),
            Some(&OsString::from("1"))
        );
    }
}
