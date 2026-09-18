//! Sparse, ordered formatting rules. A whole row/range is always one rule.
use super::{view::CsvColumnConfig, AppError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CellStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<BorderStyle>,
    /// Openpyxl-compatible formatting, in points and Excel format codes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excel: Option<super::excel::ExcelStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BorderStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StyleCondition {
    pub column_id: String,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StyleRule {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<StyleCondition>,
    pub style: CellStyle,
}

pub(super) fn color_valid(value: &str) -> bool {
    (value.starts_with('#')
        && [4, 5, 7, 9].contains(&value.len())
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit()))
        || [
            "default",
            "text",
            "secondary",
            "accent",
            "success",
            "warning",
            "error",
            "surface",
            "subtle",
            "accent-soft",
            "success-soft",
            "warning-soft",
            "error-soft",
            "border",
            "transparent",
        ]
        .contains(&value)
}

pub(super) fn validate(
    rules: &[StyleRule],
    columns: &BTreeMap<String, CsvColumnConfig>,
) -> Result<(), AppError> {
    let mut ids = HashSet::new();
    let valid = rules.len() <= 10000
        && rules.iter().all(|rule| {
            let style = &rule.style;
            !rule.id.trim().is_empty()
                && rule.id.len() <= 128
                && !rule.id.chars().any(char::is_control)
                && ids.insert(&rule.id)
                && rule
                    .rows
                    .is_none_or(|[start, end]| start <= end && end < 500_000)
                && rule.columns.as_ref().is_none_or(|ids| {
                    !ids.is_empty()
                        && ids.iter().collect::<HashSet<_>>().len() == ids.len()
                        && ids.iter().all(|id| columns.contains_key(id))
                })
                && rule.when.as_ref().is_none_or(|condition| {
                    if !columns.contains_key(&condition.column_id) {
                        return false;
                    }
                    match condition.op.as_str() {
                        "empty" | "not_empty" => condition.value.is_none(),
                        "eq" | "ne" | "contains" | "not_contains" => condition
                            .value
                            .as_ref()
                            .is_some_and(|v| v.is_string() || v.is_number()),
                        "gt" | "gte" | "lt" | "lte" | "num_eq" | "num_ne" => condition
                            .value
                            .as_ref()
                            .and_then(|v| {
                                v.as_f64()
                                    .or_else(|| v.as_str()?.trim().parse::<f64>().ok())
                            })
                            .is_some_and(f64::is_finite),
                        _ => false,
                    }
                })
                && style != &CellStyle::default()
                && style.font.as_ref().is_none_or(|font| {
                    !font.trim().is_empty()
                        && font.len() <= 256
                        && !font
                            .chars()
                            .any(|ch| ch.is_control() || ";:{}\\(),<>\"'".contains(ch))
                })
                && style.size.is_none_or(|size| (8..=72).contains(&size))
                && style.color.as_deref().is_none_or(color_valid)
                && style.background.as_deref().is_none_or(color_valid)
                && style.excel.as_ref().is_none_or(super::excel::valid_style)
                && style.border.as_ref().is_none_or(|border| {
                    border.color.as_deref().is_none_or(color_valid)
                        && border.width.is_none_or(|width| width <= 4)
                        && border.style.as_deref().is_none_or(|style| {
                            ["solid", "dashed", "dotted", "double", "none"].contains(&style)
                        })
                        && border.edges.as_ref().is_none_or(|edges| {
                            !edges.is_empty()
                                && edges.iter().collect::<HashSet<_>>().len() == edges.len()
                                && edges.iter().all(|edge| {
                                    ["top", "right", "bottom", "left"].contains(&edge.as_str())
                                })
                        })
                })
        });
    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            "csv.invalid_style",
            "Invalid CSV style rule, range, condition or formatting value.",
        ))
    }
}

pub(super) fn reconcile(rules: &mut Vec<StyleRule>, columns: &BTreeMap<String, CsvColumnConfig>) {
    rules.retain_mut(|rule| {
        if rule
            .when
            .as_ref()
            .is_some_and(|when| !columns.contains_key(&when.column_id))
        {
            return false;
        }
        if let Some(ids) = &mut rule.columns {
            ids.retain(|id| columns.contains_key(id));
            if ids.is_empty() {
                return false;
            }
        }
        true
    });
}
