use super::{data::CsvShape, AppError, CSV_VIEW_LIMIT};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CsvViewConfig {
    pub schema: String,
    pub header_rows: u8,
    pub row_height: u16,
    pub wrap_text: bool,
    pub frozen_columns: usize,
    pub columns: BTreeMap<String, CsvColumnConfig>,
    pub column_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<CsvSortConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<CsvFilterConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub styles: Vec<super::styles::StyleRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merges: Vec<super::merges::CsvMerge>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub row_dimensions: BTreeMap<String, CsvRowDimension>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CsvRowDimension {
    #[serde(serialize_with = "super::excel::serialize_number")]
    pub height: f64,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CsvColumnConfig {
    pub source_index: usize,
    pub header: String,
    pub width: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CsvSortConfig {
    pub column_id: String,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CsvFilterConfig {
    pub column_id: String,
    pub value: String,
}

impl Default for CsvViewConfig {
    fn default() -> Self {
        Self {
            schema: "locus.csv-view.v1".into(),
            header_rows: 1,
            row_height: 28,
            wrap_text: false,
            frozen_columns: 0,
            columns: BTreeMap::new(),
            column_order: vec![],
            sort: vec![],
            filters: vec![],
            styles: vec![],
            merges: vec![],
            row_dimensions: BTreeMap::new(),
        }
    }
}

pub(crate) fn parse_view(content: &str) -> Result<CsvViewConfig, AppError> {
    if content.len() > CSV_VIEW_LIMIT {
        return Err(AppError::new(
            "csv.view_too_large",
            "The CSV view file exceeds 1 MiB.",
        ));
    }
    // The Value pass detects duplicate mapping keys (including inside columns)
    // before deserializing into maps, which would otherwise keep the last value.
    let value: serde_yaml::Value = serde_yaml::from_str(content)
        .map_err(|error| AppError::new("csv.invalid_view", error.to_string()))?;
    let view: CsvViewConfig = serde_yaml::from_value(value)
        .map_err(|error| AppError::new("csv.invalid_view", error.to_string()))?;
    view.validate()?;
    Ok(view)
}

impl CsvViewConfig {
    pub fn validate(&self) -> Result<(), AppError> {
        let mut indices = HashSet::new();
        let order: HashSet<_> = self.column_order.iter().collect();
        let valid = matches!(
            self.schema.as_str(),
            "locus.csv-view.v1" | "locus.csv-view.v2" | "locus.csv-view.v3" | "locus.csv-view.v4"
        ) && (self.styles.is_empty() || self.schema != "locus.csv-view.v1")
            && (self.merges.is_empty() || self.schema >= "locus.csv-view.v3".to_string())
            && ((self.row_dimensions.is_empty()
                && !self.styles.iter().any(|r| {
                    r.style.excel.is_some()
                        || r.when.as_ref().is_some_and(|w| w.op.starts_with("num_"))
                }))
                || self.schema == "locus.csv-view.v4")
            && self.row_dimensions.len() <= 10000
            && self.row_dimensions.iter().all(|(row, dim)| {
                row.parse::<usize>()
                    .is_ok_and(|n| n < 500_000 && n.to_string() == *row)
                    && dim.height.is_finite()
                    && (1.0..=409.0).contains(&dim.height)
            })
            && self.header_rows <= 1
            && (20..=120).contains(&self.row_height)
            && self.frozen_columns <= 10000
            && self.columns.len() <= 10000
            && order.len() == self.columns.len()
            && self.column_order.len() == self.columns.len()
            && order.iter().all(|key| self.columns.contains_key(*key))
            && self.columns.iter().all(|(id, column)| {
                id.len() <= 101
                    && id.starts_with(|char: char| char.is_ascii_alphabetic())
                    && id
                        .chars()
                        .all(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '-'))
                    && !matches!(id.as_str(), "constructor" | "prototype" | "__proto__")
                    && column.source_index < 10000
                    && indices.insert(column.source_index)
                    && (48..=2000).contains(&column.width)
            })
            && self.sort.iter().all(|entry| {
                self.columns.contains_key(&entry.column_id)
                    && matches!(entry.direction.as_str(), "asc" | "desc")
            })
            && self
                .filters
                .iter()
                .all(|entry| self.columns.contains_key(&entry.column_id));
        if !valid {
            return Err(AppError::new(
                "csv.invalid_view",
                "Invalid CSV view configuration.",
            ));
        }
        super::styles::validate(&self.styles, &self.columns)?;
        super::merges::validate(&self.merges)
    }

    pub fn serialize(&self) -> Result<String, AppError> {
        self.validate()?;
        // Match the editor's deterministic YAML layout, including quoted string
        // values and indented sequences, so switching writers does not churn diffs.
        let quote = |text: &str| {
            serde_json::to_string(text)
                .unwrap()
                .replace('\u{85}', "\\u0085")
                .replace('\u{2028}', "\\u2028")
                .replace('\u{2029}', "\\u2029")
        };
        let mut text = format!(
            "schema: {}\nheaderRows: {}\nrowHeight: {}\nwrapText: {}\nfrozenColumns: {}\ncolumns:",
            quote(&self.schema),
            self.header_rows,
            self.row_height,
            self.wrap_text,
            self.frozen_columns
        );
        if self.columns.is_empty() {
            text.push_str(" {}\n");
        } else {
            text.push('\n');
            for (id, column) in &self.columns {
                let id = if ["null", "true", "false"].contains(&id.to_ascii_lowercase().as_str()) {
                    quote(id)
                } else {
                    id.clone()
                };
                text.push_str(&format!(
                    "  {id}:\n    sourceIndex: {}\n    header: {}\n    width: {}\n",
                    column.source_index,
                    quote(&column.header),
                    column.width
                ));
                if column.hidden {
                    text.push_str("    hidden: true\n");
                }
            }
        }
        text.push_str("columnOrder:");
        if self.column_order.is_empty() {
            text.push_str(" []\n");
        } else {
            text.push('\n');
            for id in &self.column_order {
                text.push_str(&format!("  - {}\n", quote(id)));
            }
        }
        if !self.sort.is_empty() {
            text.push_str("sort:\n");
            for item in &self.sort {
                text.push_str(&format!(
                    "  - columnId: {}\n    direction: {}\n",
                    quote(&item.column_id),
                    quote(&item.direction)
                ));
            }
        }
        if !self.filters.is_empty() {
            text.push_str("filters:\n");
            for item in &self.filters {
                text.push_str(&format!(
                    "  - columnId: {}\n    value: {}\n",
                    quote(&item.column_id),
                    quote(&item.value)
                ));
            }
        }
        if !self.styles.is_empty() {
            text.push_str("styles:\n");
            for rule in &self.styles {
                // One compact flow mapping per rule, not per matching cell.
                text.push_str("  - ");
                text.push_str(
                    &serde_json::to_string(rule)
                        .unwrap()
                        .replace('\u{85}', "\\u0085")
                        .replace('\u{2028}', "\\u2028")
                        .replace('\u{2029}', "\\u2029"),
                );
                text.push('\n');
            }
        }
        if !self.merges.is_empty() {
            text.push_str("merges:\n");
            for merge in &self.merges {
                text.push_str(&format!("  - {}\n", serde_json::to_string(merge).unwrap()));
            }
        }
        parse_view(&text)?;
        if !self.row_dimensions.is_empty() {
            text.push_str("rowDimensions:\n");
            let mut dimensions: Vec<_> = self.row_dimensions.iter().collect();
            dimensions.sort_by_key(|(row, _)| row.parse::<usize>().unwrap());
            for (row, dim) in dimensions {
                text.push_str(&format!(
                    "  \"{row}\": {}\n",
                    serde_json::to_string(dim).unwrap()
                ));
            }
        }
        Ok(text)
    }

    /// Keep the editor's header/position binding rules. IDs for new columns are
    /// deterministic until the first save, so two read-only calls agree.
    pub(super) fn reconcile(&mut self, shape: &CsvShape) {
        let count = shape.column_count.max(1).max(
            self.columns
                .values()
                .map(|c| c.source_index + 1)
                .max()
                .unwrap_or(0),
        );
        let headers: Vec<_> = (0..count)
            .map(|index| {
                if self.header_rows == 1 {
                    shape.headers.get(index).cloned().unwrap_or_default()
                } else {
                    String::new()
                }
            })
            .collect();
        let natural_order = self
            .column_order
            .windows(2)
            .all(|ids| self.columns[&ids[0]].source_index < self.columns[&ids[1]].source_index);
        let mut available: HashSet<_> = self.columns.keys().cloned().collect();
        let mut columns = BTreeMap::new();
        let mut keys = Vec::new();
        for (index, header) in headers.iter().enumerate() {
            let matching: Vec<_> = self
                .columns
                .iter()
                .filter(|(id, col)| available.contains(*id) && col.header == *header)
                .map(|(id, _)| id.clone())
                .collect();
            let id = matching
                .iter()
                .find(|id| self.columns[*id].source_index == index)
                .cloned()
                .or_else(|| {
                    (!header.is_empty()
                        && matching.len() == 1
                        && headers.iter().filter(|h| *h == header).count() == 1)
                        .then(|| matching[0].clone())
                })
                .or_else(|| {
                    self.columns
                        .iter()
                        .find(|(id, col)| {
                            available.contains(*id)
                                && col.source_index == index
                                && (col.header.is_empty() || !headers.contains(&col.header))
                        })
                        .map(|(id, _)| id.clone())
                });
            let (key, mut column) = match id {
                Some(id) => {
                    available.remove(&id);
                    let column = self.columns[&id].clone();
                    (id, column)
                }
                None => {
                    let mut key = format!("c_{index}");
                    let mut suffix = 0;
                    while self.columns.contains_key(&key) || columns.contains_key(&key) {
                        suffix += 1;
                        key = format!("c_{index}_{suffix}");
                    }
                    (
                        key,
                        CsvColumnConfig {
                            source_index: index,
                            header: header.clone(),
                            width: 140,
                            hidden: false,
                        },
                    )
                }
            };
            column.source_index = index;
            column.header = header.clone();
            columns.insert(key.clone(), column);
            keys.push(key);
        }
        self.column_order.retain(|id| columns.contains_key(id));
        for id in keys {
            if !self.column_order.contains(&id) {
                self.column_order.push(id);
            }
        }
        if natural_order {
            self.column_order.sort_by_key(|id| columns[id].source_index);
        }
        self.frozen_columns = self.frozen_columns.min(count);
        self.sort
            .retain(|item| columns.contains_key(&item.column_id));
        self.filters
            .retain(|item| columns.contains_key(&item.column_id));
        self.columns = columns;
        super::styles::reconcile(&mut self.styles, &self.columns);
    }

    /// v1 -> v2 only adds optional sparse style rules; existing layout is kept.
    /// Reads never migrate on disk. Applying formatting upgrades once on save.
    pub fn migrate_to_v2(&mut self) {
        if self.schema == "locus.csv-view.v1" {
            self.schema = "locus.csv-view.v2".into();
        }
    }

    /// Adds optional source rectangles without altering data, styles or layout.
    pub fn migrate_to_v3(&mut self) {
        if self.schema != "locus.csv-view.v4" {
            self.schema = "locus.csv-view.v3".into();
        }
    }

    pub fn migrate_to_v4(&mut self) {
        self.schema = "locus.csv-view.v4".into();
    }
}
