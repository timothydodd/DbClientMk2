//! Text renderings of result rows: tab-separated (clipboard), CSV, JSON and INSERT statements.
//! A single copied NULL cell is the literal `NULL`; multi-cell copies and CSV write NULL as empty;
//! JSON writes `null`. `xlsx` writes several result sets as worksheets of one workbook.

use crate::sql_identifier::{quote, SqlDialect};
use crate::update_sql::{is_numeric_type, literal, NULL_LITERAL};

pub fn single_cell(value: Option<&str>) -> String {
    value.unwrap_or(NULL_LITERAL).to_string()
}

pub fn tsv(headers: &[String], rows: &[Vec<Option<String>>], with_headers: bool) -> String {
    let mut out = String::new();
    if with_headers {
        out.push_str(&headers.join("\t"));
        out.push('\n');
    }
    for row in rows {
        let line: Vec<&str> = row.iter().map(|v| v.as_deref().unwrap_or("")).collect();
        out.push_str(&line.join("\t"));
        out.push('\n');
    }
    out
}

fn csv_escape(value: Option<&str>) -> String {
    match value {
        None => String::new(),
        Some(v) if v.contains(',') || v.contains('"') || v.contains('\n') => format!("\"{}\"", v.replace('"', "\"\"")),
        Some(v) => v.to_string(),
    }
}

pub fn csv(headers: &[String], rows: &[Vec<Option<String>>]) -> String {
    let mut out = String::new();
    out.push_str(&headers.iter().map(|h| csv_escape(Some(h))).collect::<Vec<_>>().join(","));
    out.push('\n');
    for row in rows {
        out.push_str(&row.iter().map(|v| csv_escape(v.as_deref())).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

/// A JSON array of objects keyed by column name; numeric columns that parse become JSON numbers.
pub fn json(headers: &[String], column_types: &[Option<String>], rows: &[Vec<Option<String>>]) -> String {
    let mut arr = Vec::with_capacity(rows.len());
    for row in rows {
        let mut obj = serde_json::Map::new();
        for (i, h) in headers.iter().enumerate() {
            let value = row.get(i).cloned().flatten();
            let numeric = column_types.get(i).and_then(|t| t.as_deref()).is_some_and(|t| is_numeric_type(Some(t)));
            let v = match value {
                None => serde_json::Value::Null,
                Some(s) => match (numeric, serde_json::from_str::<serde_json::Number>(s.trim())) {
                    (true, Ok(n)) => serde_json::Value::Number(n),
                    _ => serde_json::Value::String(s),
                },
            };
            obj.insert(h.clone(), v);
        }
        arr.push(serde_json::Value::Object(obj));
    }
    serde_json::to_string_pretty(&serde_json::Value::Array(arr)).unwrap_or_else(|_| "[]".into())
}

/// `quoted_table` is already quoted (or a placeholder like `<table>`).
pub fn inserts(dialect: SqlDialect, quoted_table: &str, headers: &[String], column_types: &[Option<String>], rows: &[Vec<Option<String>>]) -> String {
    let columns = headers.iter().map(|h| quote(dialect, h)).collect::<Vec<_>>().join(", ");
    let mut out = String::new();
    for row in rows {
        let values: Vec<String> = (0..headers.len())
            .map(|i| literal(row.get(i).and_then(|v| v.as_deref()), column_types.get(i).and_then(|t| t.as_deref())))
            .collect();
        out.push_str(&format!("INSERT INTO {quoted_table} ({columns}) VALUES ({});\n", values.join(", ")));
    }
    out
}

/// One result set to write as a worksheet.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XlsxSheet {
    pub name: String,
    pub column_names: Vec<String>,
    pub column_types: Vec<Option<String>>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// Excel's limits: 31-char sheet names without `[]:*?/\`, 32767 chars per cell, 1,048,576 rows.
const MAX_SHEET_NAME: usize = 31;
const MAX_CELL_CHARS: usize = 32_767;
const MAX_ROWS: usize = 1_048_576;

fn sheet_name(raw: &str, index: usize, used: &mut Vec<String>) -> String {
    let cleaned: String = raw.chars().filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\')).collect();
    let cleaned = cleaned.trim().trim_matches('\'').to_string();
    let base = if cleaned.is_empty() { format!("Result {}", index + 1) } else { cleaned };
    let mut name: String = base.chars().take(MAX_SHEET_NAME).collect();
    let mut n = 2;
    while used.iter().any(|u| u.eq_ignore_ascii_case(&name)) {
        let suffix = format!(" ({n})");
        name = base.chars().take(MAX_SHEET_NAME - suffix.len()).collect::<String>() + &suffix;
        n += 1;
    }
    used.push(name.clone());
    name
}

/// A numeric column's value as an f64, unless Excel would lose digits (more than 15 significant).
fn excel_number(value: &str) -> Option<f64> {
    let t = value.trim();
    let digits = t.trim_start_matches(['-', '+']).trim_start_matches(['0', '.']).chars().filter(|c| c.is_ascii_digit()).count();
    if digits > 15 || t.contains(['e', 'E']) {
        return None;
    }
    t.parse::<f64>().ok().filter(|f| f.is_finite())
}

fn truncate_cell(value: &str) -> &str {
    match value.char_indices().nth(MAX_CELL_CHARS) {
        Some((i, _)) => &value[..i],
        None => value,
    }
}

/// An .xlsx workbook with one worksheet per sheet: bold frozen header row, numeric columns written
/// as numbers, NULL as an empty cell.
pub fn xlsx(sheets: &[XlsxSheet]) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{Format, Workbook};
    let err = |e: rust_xlsxwriter::XlsxError| e.to_string();
    let mut workbook = Workbook::new();
    let bold = Format::new().set_bold();
    let mut used = Vec::new();
    for (index, sheet) in sheets.iter().enumerate() {
        let ws = workbook.add_worksheet();
        ws.set_name(sheet_name(&sheet.name, index, &mut used)).map_err(err)?;
        for (c, h) in sheet.column_names.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, truncate_cell(h), &bold).map_err(err)?;
        }
        let numeric: Vec<bool> = (0..sheet.column_names.len())
            .map(|i| is_numeric_type(sheet.column_types.get(i).and_then(|t| t.as_deref())))
            .collect();
        for (r, row) in sheet.rows.iter().take(MAX_ROWS - 1).enumerate() {
            let r = (r + 1) as u32;
            for (c, value) in row.iter().enumerate() {
                let Some(v) = value.as_deref() else { continue };
                match numeric.get(c).copied().unwrap_or(false).then(|| excel_number(v)).flatten() {
                    Some(n) => ws.write_number(r, c as u16, n).map_err(err)?,
                    None => ws.write_string(r, c as u16, truncate_cell(v)).map_err(err)?,
                };
            }
        }
        if !sheet.column_names.is_empty() {
            ws.set_freeze_panes(1, 0).map_err(err)?;
            ws.set_autofit_max_width(400).set_autofit_max_row(1000).autofit();
        }
    }
    if sheets.is_empty() {
        workbook.add_worksheet();
    }
    workbook.save_to_buffer().map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Vec<Option<String>>> {
        vec![vec![Some("1".into()), Some("a,b".into()), None]]
    }
    fn headers() -> Vec<String> {
        vec!["id".into(), "name".into(), "note".into()]
    }

    #[test]
    fn csv_quotes_and_nulls() {
        assert_eq!(csv(&headers(), &rows()), "id,name,note\n1,\"a,b\",\n");
    }

    #[test]
    fn tsv_with_headers() {
        assert_eq!(tsv(&headers(), &rows(), true), "id\tname\tnote\n1\ta,b\t\n");
    }

    #[test]
    fn json_numbers_and_nulls() {
        let types = vec![Some("int".into()), Some("varchar".into()), None];
        let j = json(&headers(), &types, &rows());
        assert!(j.contains("\"id\": 1"));
        assert!(j.contains("\"note\": null"));
    }

    #[test]
    fn insert_statements() {
        let types = vec![Some("int".into()), Some("varchar".into()), None];
        let s = inserts(SqlDialect::SqlServer, "[t]", &headers(), &types, &rows());
        assert_eq!(s, "INSERT INTO [t] ([id], [name], [note]) VALUES (1, 'a,b', NULL);\n");
    }

    #[test]
    fn sheet_names_are_cleaned_and_unique() {
        let mut used = Vec::new();
        assert_eq!(sheet_name("Result 1", 0, &mut used), "Result 1");
        assert_eq!(sheet_name("result 1", 1, &mut used), "result 1 (2)");
        assert_eq!(sheet_name("a/b:c", 2, &mut used), "abc");
        assert_eq!(sheet_name("", 3, &mut used), "Result 4");
        assert_eq!(sheet_name(&"x".repeat(40), 4, &mut used).len(), 31);
    }

    #[test]
    fn excel_numbers_keep_precision() {
        assert_eq!(excel_number("42"), Some(42.0));
        assert_eq!(excel_number("-0.0012"), Some(-0.0012));
        assert_eq!(excel_number("12345678901234567890"), None);
        assert_eq!(excel_number("abc"), None);
    }

    #[test]
    fn xlsx_writes_a_zip_with_every_sheet() {
        let types = vec![Some("int".into()), Some("varchar".into()), None];
        let sheet = |name: &str| XlsxSheet { name: name.into(), column_names: headers(), column_types: types.clone(), rows: rows() };
        let bytes = xlsx(&[sheet("Result 1"), sheet("Result 2")]).unwrap();
        assert_eq!(&bytes[..2], b"PK");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("xl/worksheets/sheet2.xml"));
    }

    #[test]
    fn single_cell_null_literal() {
        assert_eq!(single_cell(None), "NULL");
        assert_eq!(single_cell(Some("")), "");
    }
}
