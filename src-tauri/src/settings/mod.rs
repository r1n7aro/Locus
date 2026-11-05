//! Locus settings: a single definition table (`definitions.rs`), sparse
//! persistence in `settings.json` (`store.rs`) and the one-time import of the
//! old `config.json` (`legacy.rs`).
//!
//! Design: `design/设置系统/设置页面与存储整理.md`.

pub mod agent;
pub mod apply;
pub mod catalog;
pub mod commands;
pub mod definitions;
mod effects;
pub mod global;
pub mod group;
pub mod legacy;
pub mod mirrors;
pub mod project;
mod schema;
pub mod store;

pub use definitions::*;
pub use schema::{Key, SettingDef, SettingValue};
pub use store::SettingsStore;
