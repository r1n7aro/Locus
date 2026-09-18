use super::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CsvMerge {
    pub rows: [usize; 2],
    pub columns: [usize; 2],
}

pub(super) fn validate(merges: &[CsvMerge]) -> Result<(), AppError> {
    let invalid = || AppError::new("csv.invalid_view", "Invalid or overlapping CSV merged ranges.");
    if merges.len() > 10000 { return Err(invalid()); }
    let (mut area, mut rows, mut columns) = (0, 0, 0);
    for merge in merges {
        if merge.rows[0] > merge.rows[1] || merge.rows[1] >= 500000
            || merge.columns[0] > merge.columns[1] || merge.columns[1] >= 10000 {
            return Err(invalid());
        }
        let size = (merge.rows[1] - merge.rows[0] + 1) * (merge.columns[1] - merge.columns[0] + 1);
        area += size;
        rows = rows.max(merge.rows[1] + 1);
        columns = columns.max(merge.columns[1] + 1);
        if size < 2 || area > 500000 || rows * columns > 500000 { return Err(invalid()); }
    }
    let mut ordered: Vec<_> = merges.iter().collect();
    ordered.sort_by_key(|merge| merge.rows[0]);
    let mut active: Vec<&CsvMerge> = vec![];
    for merge in ordered {
        active.retain(|other| other.rows[1] >= merge.rows[0]);
        if active.iter().any(|other| other.columns[0] <= merge.columns[1] && merge.columns[0] <= other.columns[1]) {
            return Err(invalid());
        }
        active.push(merge);
    }
    Ok(())
}
