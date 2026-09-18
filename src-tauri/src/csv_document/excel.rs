//! The display subset of openpyxl. Protection, printing and calculation are not CSV features.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExcelStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<Font>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Fill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<BTreeMap<String, Side>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Font {
    pub name: String,
    #[serde(serialize_with = "serialize_number")]
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: String,
    pub color: String,
    pub vert_align: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Fill {
    pub pattern_type: String,
    pub fg_color: String,
    pub bg_color: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Side {
    pub style: String,
    pub color: String,
}

pub(super) fn serialize_number<S: serde::Serializer>(
    value: &f64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Alignment {
    pub horizontal: String,
    pub vertical: String,
    pub wrap_text: bool,
    pub shrink_to_fit: bool,
    pub text_rotation: u16,
    pub indent: u8,
    pub reading_order: u8,
}

pub(super) fn valid_style(value: &ExcelStyle) -> bool {
    let color = super::styles::color_valid;
    value != &ExcelStyle::default()
        && value.font.as_ref().is_none_or(|v| {
            !v.name.trim().is_empty()
                && v.name.len() <= 256
                && !v
                    .name
                    .chars()
                    .any(|ch| ch.is_control() || ";:{}\\(),<>\"'".contains(ch))
                && v.size.is_finite()
                && (1.0..=409.0).contains(&v.size)
                && [
                    "none",
                    "single",
                    "double",
                    "singleAccounting",
                    "doubleAccounting",
                ]
                .contains(&v.underline.as_str())
                && ["baseline", "superscript", "subscript"].contains(&v.vert_align.as_str())
                && color(&v.color)
        })
        && value.fill.as_ref().is_none_or(|v| {
            [
                "none",
                "solid",
                "darkDown",
                "darkGray",
                "darkGrid",
                "darkHorizontal",
                "darkTrellis",
                "darkUp",
                "darkVertical",
                "gray0625",
                "gray125",
                "lightDown",
                "lightGray",
                "lightGrid",
                "lightHorizontal",
                "lightTrellis",
                "lightUp",
                "lightVertical",
                "mediumGray",
            ]
            .contains(&v.pattern_type.as_str())
                && color(&v.fg_color)
                && color(&v.bg_color)
        })
        && value.border.as_ref().is_none_or(|v| {
            v.iter().all(|(edge, side)| {
                ["top", "right", "bottom", "left"].contains(&edge.as_str())
                    && [
                        "none",
                        "thin",
                        "medium",
                        "thick",
                        "hair",
                        "dashed",
                        "dotted",
                        "double",
                        "dashDot",
                        "dashDotDot",
                        "mediumDashed",
                        "mediumDashDot",
                        "mediumDashDotDot",
                        "slantDashDot",
                    ]
                    .contains(&side.style.as_str())
                    && color(&side.color)
            })
        })
        && value.alignment.as_ref().is_none_or(|v| {
            [
                "general",
                "left",
                "center",
                "right",
                "justify",
                "distributed",
            ]
            .contains(&v.horizontal.as_str())
                && ["top", "center", "bottom", "justify", "distributed"]
                    .contains(&v.vertical.as_str())
                && (v.text_rotation <= 180 || v.text_rotation == 255)
                && v.reading_order <= 2
        })
        && value
            .number_format
            .as_ref()
            .is_none_or(|v| !v.is_empty() && v.len() <= 512 && !v.chars().any(char::is_control))
}
