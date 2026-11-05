//! In-process mirrors of settings that hot paths read without a config
//! handle, plus the `settings-changed` broadcast. Installed once on the app's
//! store, so every write path (generic `settings_set`, `config_set`, the
//! legacy per-setting commands, `AppConfig` setters) keeps them current.
//! Unit tests never install it and therefore never touch these globals.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::definitions as keys;
use super::store::{SettingChange, SettingsStore};

pub const SETTINGS_CHANGED_EVENT: &str = "settings-changed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChangedEvent<'a> {
    pub changes: &'a [SettingChange],
}

pub fn install(store: &SettingsStore, app: AppHandle, debug_flag: Arc<AtomicBool>) {
    store.add_listener(Box::new(move |store, changes| {
        for change in changes {
            mirror(store, change, &debug_flag);
        }
        if let Err(error) = app.emit(SETTINGS_CHANGED_EVENT, SettingsChangedEvent { changes }) {
            eprintln!("[Locus] failed to publish settings change: {error}");
        }
    }));
}

fn mirror(store: &SettingsStore, change: &SettingChange, debug_flag: &AtomicBool) {
    let enabled = change.new.as_bool().unwrap_or(false);
    match change.key {
        key if key == keys::SYSTEM_DEBUG.name() => debug_flag.store(enabled, Ordering::Relaxed),
        key if key == keys::AGENT_LLM_RETRY_MAX_ATTEMPTS.name() => {
            crate::llm::retry::set_max_retries(store.get(keys::AGENT_LLM_RETRY_MAX_ATTEMPTS))
        }
        key if key == keys::AGENT_LLM_STRIP_INLINE_THINK_TAGS.name() => {
            crate::llm::think_tag_filter::set_enabled(enabled)
        }
        key if key == keys::UNITY_ENABLED.name() => {
            crate::unity_bridge::set_unity_integration_enabled(enabled)
        }
        key if key == keys::UNITY_MULTI_AGENT_EDITOR.name() => {
            crate::unity_editor_lock::set_enabled(enabled)
        }
        key if key == keys::BLENDER_ENABLED.name() => crate::blender::switch_changed(enabled),
        key if key == keys::UNITY_COMPILE_NON_PUBLIC_ACCESS.name() => {
            crate::csharp_compile::set_non_public_access_enabled(enabled)
        }
        key if key == keys::UNITY_COMPILE_IN_PROCESS_FALLBACK.name() => {
            crate::csharp_compile::set_in_process_fallback(enabled)
        }
        key if key == keys::UNITY_HOT_RELOAD_ENABLED.name() => {
            crate::unity_hotreload::set_enabled(enabled)
        }
        key if key == keys::UNITY_HOT_RELOAD_INLINE_FORCE_EVALUATE.name() => {
            crate::unity_hotreload::set_inline_force_evaluate_enabled(enabled)
        }
        key if key.starts_with("unity.codeAnalysis.tools.") => {
            crate::code_tools::set(crate::config::CodeAnalysisToolsConfig::from_settings(store))
        }
        _ => {}
    }
}
