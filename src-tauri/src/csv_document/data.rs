//! Read only the shape/header needed for view binding. Never serialize CSV data.
use super::AppError;

pub(super) struct CsvShape {
    pub headers: Vec<String>,
    pub row_count: usize,
    pub column_count: usize,
    pub delimiter: u8,
}

fn invalid(offset: usize) -> AppError {
    AppError::new(
        "csv.invalid_csv",
        format!("Invalid CSV quoting at byte {offset}."),
    )
}

fn scan(source: &str, delimiter: u8, preview: bool) -> Result<(CsvShape, Vec<usize>), AppError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let bytes = source.as_bytes();
    let mut position = 0;
    let mut shape = CsvShape {
        headers: Vec::new(),
        row_count: 0,
        column_count: 0,
        delimiter,
    };
    let mut widths = Vec::new();
    while position < bytes.len() {
        let mut width = 0;
        loop {
            let start = position;
            let mut value = None;
            if bytes.get(position) == Some(&b'"') {
                position += 1;
                let value_start = position;
                loop {
                    match bytes.get(position) {
                        None => return Err(invalid(start)),
                        Some(b'"') if bytes.get(position + 1) == Some(&b'"') => position += 2,
                        Some(b'"') => break,
                        _ => position += 1,
                    }
                }
                if shape.row_count == 0 {
                    value = Some(source[value_start..position].replace("\"\"", "\""));
                }
                position += 1;
                if bytes
                    .get(position)
                    .is_some_and(|b| ![delimiter, b'\r', b'\n'].contains(b))
                {
                    return Err(invalid(position));
                }
            } else {
                while bytes
                    .get(position)
                    .is_some_and(|b| ![delimiter, b'\r', b'\n'].contains(b))
                {
                    if bytes[position] == b'"' && !preview {
                        return Err(invalid(position));
                    }
                    position += 1;
                }
                if shape.row_count == 0 {
                    value = Some(source[start..position].to_owned());
                }
            }
            if let Some(value) = value {
                shape.headers.push(value);
            }
            width += 1;
            if width > 10000 {
                return Err(AppError::new(
                    "csv.too_large",
                    "CSV exceeds 10,000 columns.",
                ));
            }
            if bytes.get(position) == Some(&delimiter) {
                position += 1;
                continue;
            }
            if bytes.get(position) == Some(&b'\r') {
                position += 1;
                if bytes.get(position) == Some(&b'\n') {
                    position += 1;
                }
            } else if bytes.get(position) == Some(&b'\n') {
                position += 1;
            }
            break;
        }
        shape.row_count += 1;
        shape.column_count = shape.column_count.max(width);
        widths.push(width);
        if shape.row_count * shape.column_count > 500_000 {
            return Err(AppError::new(
                "csv.too_large",
                "CSV exceeds 500,000 rectangular cells.",
            ));
        }
        if preview && widths.len() == 10 {
            break;
        }
    }
    // Papa's delimiter preview counts the terminal empty line; the editor's
    // lexical document parser does not turn it into an extra source record.
    if preview
        && widths.len() < 10
        && (bytes.is_empty() || matches!(bytes.last(), Some(b'\n' | b'\r')))
    {
        widths.push(1);
    }
    Ok((shape, widths))
}

pub(super) fn parse_shape(source: &str) -> Result<CsvShape, AppError> {
    if source.contains('\0') {
        return Err(AppError::new(
            "csv.invalid_encoding",
            "CSV must be UTF-8 text without NUL bytes.",
        ));
    }
    let mut delimiter = b',';
    let mut best: Option<(usize, f64)> = None;
    // Same candidate order and score as the editor's Papa delimiter detection.
    for candidate in [b',', b';', b'\t', b'|'] {
        if let Ok((_, widths)) = scan(source, candidate, true) {
            let average = widths.iter().sum::<usize>() as f64 / widths.len().max(1) as f64;
            let delta = widths
                .windows(2)
                .map(|pair| pair[0].abs_diff(pair[1]))
                .sum::<usize>();
            if average > 1.99 && best.is_none_or(|(d, a)| delta < d || (delta == d && average > a))
            {
                best = Some((delta, average));
                delimiter = candidate;
            }
        }
    }
    scan(source, delimiter, false).map(|(shape, _)| shape)
}
