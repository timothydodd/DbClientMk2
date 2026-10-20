//! Text renderings of result rows: tab-separated (clipboard), CSV, JSON and INSERT statements.
//! A single copied NULL cell is the literal `NULL`; multi-cell copies and CSV write NULL as empty;
//! JSON writes `null`.

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
    fn single_cell_null_literal() {
        assert_eq!(single_cell(None), "NULL");
        assert_eq!(single_cell(Some("")), "");
    }
}
