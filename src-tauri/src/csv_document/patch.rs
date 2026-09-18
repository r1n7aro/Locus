use super::styles::{CellStyle, StyleCondition, StyleRule};
use super::{
    data::CsvShape,
    view::{CsvFilterConfig, CsvSortConfig},
    AppError, CsvViewConfig,
};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColumnTarget {
    id: Option<String>,
    source_index: Option<usize>,
    header: Option<String>,
}

impl ColumnTarget {
    fn resolve(&self, view: &CsvViewConfig) -> Result<String, AppError> {
        if [
            self.id.is_some(),
            self.source_index.is_some(),
            self.header.is_some(),
        ]
        .into_iter()
        .filter(|v| *v)
        .count()
            != 1
        {
            return Err(AppError::new(
                "csv.invalid_column",
                "Specify exactly one of id, source_index or header.",
            ));
        }
        let matches: Vec<_> = view
            .columns
            .iter()
            .filter(|(id, col)| {
                self.id.as_ref().is_some_and(|v| v == *id)
                    || self.source_index.is_some_and(|v| v == col.source_index)
                    || self.header.as_ref().is_some_and(|v| v == &col.header)
            })
            .map(|(id, _)| id.clone())
            .collect();
        match matches.as_slice() {
            [id] => Ok(id.clone()),
            [] => Err(AppError::new(
                "csv.column_not_found",
                "Column not found. Read the view and use a returned column ID or source_index.",
            )),
            _ => Err(AppError::new(
                "csv.ambiguous_column",
                "The header matches multiple columns. Use a column ID or source_index.",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColumnPatch {
    target: ColumnTarget,
    width: Option<u16>,
    hidden: Option<bool>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SortPatch {
    target: ColumnTarget,
    direction: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterPatch {
    target: ColumnTarget,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConditionPatch {
    target: ColumnTarget,
    op: String,
    value: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleRulePatch {
    id: String,
    rows: Option<[usize; 2]>,
    columns: Option<Vec<ColumnTarget>>,
    when: Option<ConditionPatch>,
    style: CellStyle,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StylesPatch {
    #[serde(default)]
    upsert: Vec<StyleRulePatch>,
    #[serde(default)]
    remove: Vec<String>,
    order: Option<Vec<String>>,
}

impl StylesPatch {
    fn apply(self, view: &mut CsvViewConfig) -> Result<(), AppError> {
        let mut touched = HashSet::new();
        for id in &self.remove {
            if !touched.insert(id.clone()) || !view.styles.iter().any(|rule| &rule.id == id) {
                return Err(AppError::new(
                    "csv.invalid_style",
                    "Style removal IDs must exist and be distinct.",
                ));
            }
        }
        let mut upserts = Vec::new();
        for item in self.upsert {
            if !touched.insert(item.id.clone()) {
                return Err(AppError::new(
                    "csv.invalid_style",
                    "A batch must modify each style ID at most once.",
                ));
            }
            let columns = item
                .columns
                .map(|items| {
                    items
                        .iter()
                        .map(|target| target.resolve(view))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?;
            let when = item
                .when
                .map(|item| {
                    Ok::<_, AppError>(StyleCondition {
                        column_id: item.target.resolve(view)?,
                        op: item.op,
                        value: item.value,
                    })
                })
                .transpose()?;
            upserts.push(StyleRule {
                id: item.id,
                rows: item.rows,
                columns,
                when,
                style: item.style,
            });
        }
        view.styles.retain(|rule| !self.remove.contains(&rule.id));
        for rule in upserts {
            if let Some(existing) = view
                .styles
                .iter_mut()
                .find(|existing| existing.id == rule.id)
            {
                *existing = rule;
            } else {
                view.styles.push(rule);
            }
        }
        if let Some(order) = self.order {
            if order.len() != view.styles.len()
                || order.iter().collect::<HashSet<_>>().len() != order.len()
                || order
                    .iter()
                    .any(|id| !view.styles.iter().any(|rule| &rule.id == id))
            {
                return Err(AppError::new(
                    "csv.invalid_style",
                    "Style order must list every remaining rule ID exactly once.",
                ));
            }
            view.styles
                .sort_by_key(|rule| order.iter().position(|id| id == &rule.id).unwrap());
        }
        if !view.styles.is_empty() {
            view.migrate_to_v2();
        }
        if view.styles.iter().any(|r| {
            r.style.excel.is_some() || r.when.as_ref().is_some_and(|w| w.op.starts_with("num_"))
        }) {
            view.migrate_to_v4();
        }
        Ok(())
    }
}

/// SDK patches use Python field names. The persisted v1 schema stays camelCase.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CsvViewPatch {
    header_rows: Option<u8>,
    row_height: Option<u16>,
    wrap_text: Option<bool>,
    frozen_columns: Option<usize>,
    columns: Option<Vec<ColumnPatch>>,
    column_order: Option<Vec<ColumnTarget>>,
    sort: Option<Vec<SortPatch>>,
    filters: Option<Vec<FilterPatch>>,
    styles: Option<StylesPatch>,
    merges: Option<Vec<super::merges::CsvMerge>>,
    row_dimensions: Option<std::collections::BTreeMap<String, super::view::CsvRowDimension>>,
    column_count: Option<usize>,
}

impl CsvViewPatch {
    pub(super) fn apply(self, view: &mut CsvViewConfig, shape: &CsvShape) -> Result<(), AppError> {
        if let Some(count) = self.column_count {
            if count == 0
                || count > 10000
                || count < shape.column_count
                || count.saturating_mul(shape.row_count) > 500_000
            {
                return Err(AppError::new(
                    "csv.invalid_patch",
                    "Invalid worksheet column count.",
                ));
            }
            let expanded = CsvShape {
                headers: shape.headers.clone(),
                row_count: shape.row_count,
                column_count: count,
                delimiter: shape.delimiter,
            };
            view.reconcile(&expanded);
        }
        // Resolve every selector against the same original snapshot, including
        // when this batch also changes header interpretation or display order.
        let mut columns = Vec::new();
        let mut seen = HashSet::new();
        for item in self.columns.unwrap_or_default() {
            let id = item.target.resolve(view)?;
            if !seen.insert(id.clone()) || (item.width.is_none() && item.hidden.is_none()) {
                return Err(AppError::new(
                    "csv.invalid_patch",
                    "Column patches must be non-empty and target distinct columns.",
                ));
            }
            columns.push((id, item.width, item.hidden));
        }
        let order = self
            .column_order
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.resolve(view))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        let sort = self
            .sort
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| {
                        Ok(CsvSortConfig {
                            column_id: item.target.resolve(view)?,
                            direction: item.direction,
                        })
                    })
                    .collect::<Result<Vec<_>, AppError>>()
            })
            .transpose()?;
        let filters = self
            .filters
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| {
                        Ok(CsvFilterConfig {
                            column_id: item.target.resolve(view)?,
                            value: item.value,
                        })
                    })
                    .collect::<Result<Vec<_>, AppError>>()
            })
            .transpose()?;
        if let Some(styles) = self.styles {
            styles.apply(view)?;
        }
        if let Some(merges) = self.merges {
            super::merges::validate(&merges)?;
            if !merges.is_empty() {
                view.migrate_to_v3();
            }
            view.merges = merges;
        }
        if let Some(dimensions) = self.row_dimensions {
            if !dimensions.is_empty() {
                view.migrate_to_v4();
            }
            view.row_dimensions = dimensions;
        }
        if let Some(value) = self.header_rows {
            view.header_rows = value;
            for column in view.columns.values_mut() {
                column.header = if value == 1 {
                    shape
                        .headers
                        .get(column.source_index)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    String::new()
                };
            }
        }
        if let Some(value) = self.row_height {
            view.row_height = value;
        }
        if let Some(value) = self.wrap_text {
            view.wrap_text = value;
        }
        if let Some(value) = self.frozen_columns {
            if value > view.columns.len() {
                return Err(AppError::new(
                    "csv.invalid_patch",
                    "frozen_columns exceeds the number of configured columns.",
                ));
            }
            view.frozen_columns = value;
        }
        for (id, width, hidden) in columns {
            let column = view.columns.get_mut(&id).unwrap();
            if let Some(value) = width {
                column.width = value;
            }
            if let Some(value) = hidden {
                column.hidden = value;
            }
        }
        if let Some(value) = order {
            view.column_order = value;
        }
        if let Some(value) = sort {
            view.sort = value;
        }
        if let Some(value) = filters {
            view.filters = value;
        }
        view.validate()
    }
}
