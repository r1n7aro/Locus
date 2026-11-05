//! Setting definition types. Each setting is declared once in
//! `definitions.rs`; the declaration yields both its metadata entry and a
//! typed key constant used by Rust callers.

use std::marker::PhantomData;

use serde::Serialize;
use serde_json::Value;

/// Where a setting is persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// User-level, `settings.json` in the persistent config directory.
    App,
    /// Project-level, `<project>/Locus/config.json`.
    Project,
}

/// Approval tier for agent writes. Every setting is readable and writable by
/// the agent; sensitive settings require user confirmation by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Sensitivity {
    Normal,
    /// Widens the agent's own authority or changes where data and
    /// credentials go.
    Sensitive,
}

/// Value type, constraints and default.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Bool {
        default: bool,
    },
    Int {
        min: i64,
        max: i64,
        default: i64,
    },
    Enum {
        values: &'static [&'static str],
        /// Accepted spellings mapped to a canonical value.
        aliases: &'static [(&'static str, &'static str)],
        default: &'static str,
    },
    Text {
        max_len: usize,
        default: &'static str,
    },
    /// Filesystem path; empty means unset.
    Path,
    StringList {
        default: &'static [&'static str],
    },
    /// Integer in range, or `null` for "not set" (the consumer's own default).
    OptionalInt {
        min: i64,
        max: i64,
    },
    /// String-keyed object whose values all have the `value` kind; default `{}`.
    Map {
        value: &'static Kind,
    },
}

impl Kind {
    pub const fn bool(default: bool) -> Self {
        Self::Bool { default }
    }

    pub const fn int(min: i64, max: i64, default: i64) -> Self {
        Self::Int { min, max, default }
    }

    pub const fn enumeration(values: &'static [&'static str], default: &'static str) -> Self {
        Self::Enum {
            values,
            aliases: &[],
            default,
        }
    }

    pub const fn text(max_len: usize, default: &'static str) -> Self {
        Self::Text { max_len, default }
    }

    pub fn default_value(&self) -> Value {
        match *self {
            Kind::Bool { default } => Value::Bool(default),
            Kind::Int { default, .. } => Value::from(default),
            Kind::Enum { default, .. } => Value::from(default),
            Kind::Text { default, .. } => Value::from(default),
            Kind::Path => Value::from(""),
            Kind::StringList { default } => {
                Value::Array(default.iter().map(|item| Value::from(*item)).collect())
            }
            Kind::OptionalInt { .. } => Value::Null,
            Kind::Map { .. } => Value::Object(serde_json::Map::new()),
        }
    }

    /// Validates `value` and returns its canonical form.
    pub fn normalize(&self, value: &Value) -> Result<Value, String> {
        match *self {
            Kind::Bool { .. } => value
                .as_bool()
                .map(Value::Bool)
                .ok_or_else(|| "expected a boolean".to_string()),
            Kind::Int { min, max, .. } => {
                let number = integer_of(value).ok_or_else(|| "expected an integer".to_string())?;
                if number < min || number > max {
                    return Err(format!("must be between {min} and {max}"));
                }
                Ok(Value::from(number))
            }
            Kind::Enum {
                values, aliases, ..
            } => {
                let text = value
                    .as_str()
                    .ok_or_else(|| "expected a string".to_string())?;
                let canonical = aliases
                    .iter()
                    .find(|(alias, _)| *alias == text)
                    .map(|(_, canonical)| *canonical)
                    .unwrap_or(text);
                if values.contains(&canonical) {
                    Ok(Value::from(canonical))
                } else {
                    Err(format!("must be one of: {}", values.join(", ")))
                }
            }
            Kind::Text { max_len, .. } => {
                let text = value
                    .as_str()
                    .ok_or_else(|| "expected a string".to_string())?;
                if text.chars().count() > max_len {
                    return Err(format!("must be at most {max_len} characters"));
                }
                Ok(Value::from(text))
            }
            Kind::Path => value
                .as_str()
                .map(|text| Value::from(text.trim()))
                .ok_or_else(|| "expected a path string".to_string()),
            Kind::StringList { .. } => {
                let items = value
                    .as_array()
                    .ok_or_else(|| "expected an array of strings".to_string())?;
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(Value::from)
                            .ok_or_else(|| "expected an array of strings".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            }
            Kind::OptionalInt { min, max } => {
                if value.is_null() {
                    return Ok(Value::Null);
                }
                Kind::Int { min, max, default: min }.normalize(value)
            }
            Kind::Map { value: item_kind } => {
                let entries = value
                    .as_object()
                    .ok_or_else(|| "expected an object".to_string())?;
                let mut normalized = serde_json::Map::with_capacity(entries.len());
                for (name, item) in entries {
                    let name = name.trim();
                    if name.is_empty() {
                        return Err("object keys must not be empty".to_string());
                    }
                    let item = item_kind
                        .normalize(item)
                        .map_err(|reason| format!("'{name}': {reason}"))?;
                    normalized.insert(name.to_string(), item);
                }
                Ok(Value::Object(normalized))
            }
        }
    }

    /// Machine-readable description used by `settings_schema` and the agent.
    pub fn describe(&self) -> Value {
        match *self {
            Kind::Bool { .. } => serde_json::json!({ "type": "bool" }),
            Kind::Int { min, max, .. } => serde_json::json!({ "type": "int", "min": min, "max": max }),
            Kind::Enum { values, .. } => serde_json::json!({ "type": "enum", "values": values }),
            Kind::Text { max_len, .. } => serde_json::json!({ "type": "text", "maxLength": max_len }),
            Kind::Path => serde_json::json!({ "type": "path" }),
            Kind::StringList { .. } => serde_json::json!({ "type": "stringList" }),
            Kind::OptionalInt { min, max } => {
                serde_json::json!({ "type": "int", "min": min, "max": max, "nullable": true })
            }
            Kind::Map { value } => serde_json::json!({ "type": "map", "valueType": value.describe() }),
        }
    }
}

fn integer_of(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    if let Some(number) = value.as_u64() {
        return i64::try_from(number).ok();
    }
    value
        .as_f64()
        .filter(|number| number.fract() == 0.0 && number.is_finite())
        .filter(|number| *number >= i64::MIN as f64 && *number <= i64::MAX as f64)
        .map(|number| number as i64)
}

/// One setting. `key` is a dot path that is also its JSON path in the
/// settings file (`agent.subagent.maxDepth`).
#[derive(Debug, Clone, Copy)]
pub struct SettingDef {
    pub key: &'static str,
    pub kind: Kind,
    pub scope: Scope,
    pub sensitivity: Sensitivity,
    pub experimental: bool,
    /// Takes effect only after restarting Locus.
    pub restart: bool,
    /// Read by the frontend before its first frame; mirrored into the
    /// webview boot cache.
    pub boot: bool,
    /// Field name inside `<project>/Locus/config.json` for project settings.
    pub project_field: Option<&'static str>,
    /// Extra validation after the kind check; returns the canonical value.
    pub validate: Option<fn(&Value) -> Result<Value, String>>,
}

impl SettingDef {
    pub const BASE: SettingDef = SettingDef {
        key: "",
        kind: Kind::Bool { default: false },
        scope: Scope::App,
        sensitivity: Sensitivity::Normal,
        experimental: false,
        restart: false,
        boot: false,
        project_field: None,
        validate: None,
    };

    pub fn default_value(&self) -> Value {
        self.kind.default_value()
    }

    pub fn normalize(&self, value: &Value) -> Result<Value, String> {
        let value = self.kind.normalize(value)?;
        match self.validate {
            Some(validate) => validate(&value),
            None => Ok(value),
        }
    }
}

/// Typed handle to a setting, generated next to its definition.
pub struct Key<T> {
    index: usize,
    name: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Key<T> {
    pub const fn new(index: usize, name: &'static str) -> Self {
        Self {
            index,
            name,
            _marker: PhantomData,
        }
    }

    pub const fn index(&self) -> usize {
        self.index
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }
}

impl<T> Clone for Key<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Key<T> {}

impl<T> std::fmt::Debug for Key<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name)
    }
}

/// Conversion between Rust values and resolved JSON setting values.
pub trait SettingValue: Sized {
    fn from_setting(value: &Value) -> Option<Self>;
    fn to_setting(&self) -> Value;
}

impl SettingValue for bool {
    fn from_setting(value: &Value) -> Option<Self> {
        value.as_bool()
    }

    fn to_setting(&self) -> Value {
        Value::Bool(*self)
    }
}

impl SettingValue for u32 {
    fn from_setting(value: &Value) -> Option<Self> {
        value.as_u64().and_then(|number| u32::try_from(number).ok())
    }

    fn to_setting(&self) -> Value {
        Value::from(*self)
    }
}

impl SettingValue for u64 {
    fn from_setting(value: &Value) -> Option<Self> {
        value.as_u64()
    }

    fn to_setting(&self) -> Value {
        Value::from(*self)
    }
}

impl SettingValue for String {
    fn from_setting(value: &Value) -> Option<Self> {
        value.as_str().map(str::to_string)
    }

    fn to_setting(&self) -> Value {
        Value::from(self.as_str())
    }
}

/// Settings without a dedicated Rust type (maps, nullable integers) are read
/// as JSON and converted by their module.
impl SettingValue for Value {
    fn from_setting(value: &Value) -> Option<Self> {
        Some(value.clone())
    }

    fn to_setting(&self) -> Value {
        self.clone()
    }
}

impl SettingValue for Vec<String> {
    fn from_setting(value: &Value) -> Option<Self> {
        value
            .as_array()?
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect()
    }

    fn to_setting(&self) -> Value {
        Value::Array(self.iter().map(|item| Value::from(item.as_str())).collect())
    }
}

/// Declares the settings table: one line per setting produces a typed
/// `Key` constant and a `DEFINITIONS` entry with the same index.
macro_rules! define_settings {
    (
        $(
            $(#[$meta:meta])*
            $name:ident : $ty:ty = $key:literal, $kind:expr $(, $field:ident = $value:expr)* ;
        )*
    ) => {
        // Every setting gets a typed key; settings that a module reads as a
        // group (`settings::group`) may never use theirs.
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[derive(Clone, Copy)]
        #[repr(usize)]
        enum SettingIndex {
            $( $name, )*
        }

        $(
            $(#[$meta])*
            #[allow(dead_code)]
            pub const $name: $crate::settings::Key<$ty> =
                $crate::settings::Key::new(SettingIndex::$name as usize, $key);
        )*

        pub static DEFINITIONS: &[$crate::settings::SettingDef] = &[
            $(
                $crate::settings::SettingDef {
                    key: $key,
                    kind: $kind,
                    $( $field: $value, )*
                    ..$crate::settings::SettingDef::BASE
                },
            )*
        ];
    };
}

pub(crate) use define_settings;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn int_rejects_out_of_range_and_non_integers() {
        let kind = Kind::int(1, 8, 1);
        assert_eq!(kind.normalize(&json!(3)).unwrap(), json!(3));
        assert_eq!(kind.normalize(&json!(4.0)).unwrap(), json!(4));
        assert!(kind.normalize(&json!(0)).is_err());
        assert!(kind.normalize(&json!(9)).is_err());
        assert!(kind.normalize(&json!(2.5)).is_err());
        assert!(kind.normalize(&json!("3")).is_err());
    }

    #[test]
    fn enum_maps_aliases_to_canonical_values() {
        let kind = Kind::Enum {
            values: &["metaTool", "native"],
            aliases: &[("meta-tool", "metaTool")],
            default: "native",
        };
        assert_eq!(kind.normalize(&json!("meta-tool")).unwrap(), json!("metaTool"));
        assert_eq!(kind.normalize(&json!("native")).unwrap(), json!("native"));
        assert!(kind.normalize(&json!("other")).is_err());
    }

    #[test]
    fn text_limits_length_in_characters() {
        let kind = Kind::text(3, "");
        assert_eq!(kind.normalize(&json!("中文字")).unwrap(), json!("中文字"));
        assert!(kind.normalize(&json!("abcd")).is_err());
    }

    #[test]
    fn optional_int_accepts_null_and_range() {
        let kind = Kind::OptionalInt { min: 10, max: 20 };
        assert_eq!(kind.default_value(), Value::Null);
        assert_eq!(kind.normalize(&Value::Null).unwrap(), Value::Null);
        assert_eq!(kind.normalize(&json!(15)).unwrap(), json!(15));
        assert!(kind.normalize(&json!(5)).is_err());
    }

    #[test]
    fn map_normalizes_each_value() {
        const MODE: Kind = Kind::Enum {
            values: &["auto", "ask"],
            aliases: &[],
            default: "auto",
        };
        let kind = Kind::Map { value: &MODE };
        assert_eq!(
            kind.normalize(&json!({ " read ": "ask" })).unwrap(),
            json!({ "read": "ask" })
        );
        assert!(kind
            .normalize(&json!({ "read": "never" }))
            .unwrap_err()
            .contains("'read'"));
        assert!(kind.normalize(&json!(["ask"])).is_err());
    }

    #[test]
    fn path_trims_whitespace() {
        assert_eq!(Kind::Path.normalize(&json!("  D:/work ")).unwrap(), json!("D:/work"));
    }
}
