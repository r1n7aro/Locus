//! `[SerializeReference]` (managed reference) registry capture.
//!
//! Unity serializes managed-reference instances into a registry at the end of
//! the host MonoBehaviour / ScriptableObject document. Format v2 (2021.2+):
//!
//! ```yaml
//!   references:
//!     version: 2
//!     RefIds:
//!     - rid: 1000
//!       type: {class: Foo, ns: Game, asm: Assembly-CSharp}
//!       data:
//!         amount: 1
//! ```
//!
//! Format v1 (2019.3 – 2021.1) keys entries by a numeric id instead:
//!
//! ```yaml
//!   references:
//!     version: 1
//!     00000000:
//!       type: {class: Foo, ns: Game, asm: Assembly-CSharp}
//!       data:
//!         amount: 1
//! ```
//!
//! The instance type is identified only by `{class, ns, asm}` — there is no
//! script GUID — so the GUID reference graph never sees these usages. The
//! parser records one [`RawManagedRef`] per registry entry during its single
//! line scan; nested type names use `/` (`Outer/Inner`) and generic types
//! carry a CLR arity suffix plus argument list (``Box`1[[System.Int32, mscorlib]]``).

use super::tokenizer::{decode_yaml_string, find_closing_brace};

/// One managed-reference registry entry captured from a YAML asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawManagedRef {
    /// fileID of the host document that owns the `references:` registry.
    pub host_file_id: i64,
    /// v2 `rid`, or the v1 numeric entry key.
    pub rid: i64,
    /// Raw class string exactly as serialized (nested `/`, generic suffix).
    pub class_name: String,
    pub namespace: String,
    pub assembly: String,
    /// 1-based line of the entry start (`- rid: N` / `NNNNNNNN:`).
    pub line: u32,
}

/// The `{class, ns, asm}` identity Unity serializes for a managed reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManagedTypeIdentity {
    pub class_name: String,
    pub namespace: String,
    pub assembly: String,
}

impl ManagedTypeIdentity {
    /// `Namespace.Outer/Inner` style label for tool output.
    pub fn display_name(&self) -> String {
        if self.namespace.is_empty() {
            self.class_name.clone()
        } else {
            format!("{}.{}", self.namespace, self.class_name)
        }
    }
}

/// Sentinel entry Unity 2019.3 – 2021.1 appends to v1 registries.
fn is_terminus(identity: &ManagedTypeIdentity) -> bool {
    identity.class_name == "Terminus"
        && identity.namespace == "UnityEngine.DMAT"
        && identity.assembly == "FAKE_ASM"
}

/// Registry entry key at `references` indent + 2: `- rid: N` (v2) or
/// `NNNNNNNN:` (v1). Returns `None` for `version:` / `RefIds:` lines.
pub(super) fn registry_entry_id(trimmed: &str, dashed: bool) -> Option<i64> {
    if dashed {
        let rest = trimmed.strip_prefix('-')?.trim_start();
        let value = rest.strip_prefix("rid:")?;
        return value.trim().parse::<i64>().ok();
    }
    let key = trimmed.strip_suffix(':')?;
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<i64>().ok()
}

/// Build a captured entry from the text after `type:`; `None` for null
/// entries (`rid: -2`, empty class), the v1 terminus sentinel, or a flow map
/// that is not yet complete (wrapped across lines — the caller retries once
/// the parser has joined the continuation).
pub(super) fn build_entry(
    host_file_id: i64,
    rid: i64,
    line: u32,
    type_value: &str,
) -> Option<RawManagedRef> {
    let identity = parse_managed_type_flow(type_value)?;
    if identity.class_name.is_empty() || is_terminus(&identity) {
        return None;
    }
    Some(RawManagedRef {
        host_file_id,
        rid,
        class_name: identity.class_name,
        namespace: identity.namespace,
        assembly: identity.assembly,
        line,
    })
}

/// Parse a `{class: X, ns: Y, asm: Z}` flow map. Values may be single- or
/// double-quoted and contain commas / brackets (generic argument lists).
pub fn parse_managed_type_flow(text: &str) -> Option<ManagedTypeIdentity> {
    let open = text.find('{')?;
    let close = find_closing_brace(text.as_bytes(), open)?;
    let inner = &text[open + 1..close];

    let mut class_name: Option<String> = None;
    let mut namespace = String::new();
    let mut assembly = String::new();
    for part in split_top_level_commas(inner) {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let decoded = if value.is_empty() {
            String::new()
        } else {
            decode_yaml_string(value)
        };
        match key.trim() {
            "class" => class_name = Some(decoded),
            "ns" => namespace = decoded,
            "asm" => assembly = decoded,
            _ => {}
        }
    }
    Some(ManagedTypeIdentity {
        class_name: class_name?,
        namespace,
        assembly,
    })
}

/// Split on commas that are outside quotes and `[...]` argument lists.
fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let bytes = inner.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
        } else if in_double {
            match b {
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'"' => in_double = false,
                _ => {}
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'[' => depth += 1,
                b']' => depth -= 1,
                b',' if depth <= 0 => {
                    parts.push(&inner[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    parts.push(&inner[start..]);
    parts
}

/// One nesting level of a serialized managed-reference class name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedClassSegment {
    pub name: String,
    /// CLR generic arity (`` `N`` suffix); 0 for non-generic types.
    pub arity: usize,
}

/// Decompose a serialized class string into nesting segments, dropping the
/// generic argument list: ``Outer/Box`1[[System.Int32, mscorlib]]`` →
/// `[Outer/0, Box/1]`. CLR `+` nesting is accepted alongside Unity's `/`.
pub fn managed_class_segments(class_name: &str) -> Vec<ManagedClassSegment> {
    let without_args = match class_name.find('[') {
        Some(index) => &class_name[..index],
        None => class_name,
    };
    without_args
        .split(['/', '+'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| match segment.split_once('`') {
            Some((name, arity)) => ManagedClassSegment {
                name: name.to_string(),
                arity: arity.trim().parse().unwrap_or(0),
            },
            None => ManagedClassSegment {
                name: segment.to_string(),
                arity: 0,
            },
        })
        .collect()
}

/// Lookup key for a serialized class string: innermost simple name,
/// ASCII-lowercased (`Outer/Box`1[[...]]` → `box`).
pub fn managed_class_key(class_name: &str) -> String {
    managed_class_segments(class_name)
        .last()
        .map(|segment| segment.name.to_ascii_lowercase())
        .unwrap_or_default()
}
