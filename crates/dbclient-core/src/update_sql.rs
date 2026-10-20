//! Writes grid edits back as UPDATE statements keyed on the primary key, wrapped in a
//! transaction that fails if a statement touches anything but one row.

use crate::models::ConnectionType;
use crate::sql_identifier::{quote, SqlDialect};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;

/// Typing this literal into a grid cell means SQL NULL (case-insensitive).
pub const NULL_LITERAL: &str = "NULL";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateScript {
    pub sql: String,
    /// Number of UPDATE statements; each is expected to affect exactly one row.
    pub expected_statements: usize,
    pub error: Option<String>,
}

impl UpdateScript {
    pub fn fail(error: impl Into<String>) -> Self {
        UpdateScript { error: Some(error.into()), ..Default::default() }
    }
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

pub fn dialect_for(kind: ConnectionType) -> SqlDialect {
    match kind {
        ConnectionType::MySql => SqlDialect::MySql,
        ConnectionType::Sqlite => SqlDialect::Sqlite,
        ConnectionType::SqlServer => SqlDialect::SqlServer,
    }
}

pub fn is_null_value(value: Option<&str>) -> bool {
    match value {
        None => true,
        Some(v) => v.eq_ignore_ascii_case(NULL_LITERAL),
    }
}

pub fn is_numeric_type(db_type: Option<&str>) -> bool {
    let Some(t) = db_type else { return false };
    if t.is_empty() {
        return false;
    }
    let t = t.to_lowercase();
    t.contains("int")
        || t.contains("decimal")
        || t.contains("numeric")
        || t.contains("float")
        || t.contains("double")
        || t.contains("real")
        || t.contains("money")
        || t.contains("number")
        || t == "bit"
        || t.contains("serial")
}

fn parses_as_number(v: &str) -> bool {
    v.parse::<f64>().is_ok_and(|f| f.is_finite())
}

/// A grid value as a SQL literal: NULL for null / the NULL literal; an unquoted number only when
/// the column is numeric and the text parses as one; otherwise a single-quoted string.
pub fn literal(value: Option<&str>, db_type: Option<&str>) -> String {
    if is_null_value(value) {
        return "NULL".into();
    }
    let value = value.unwrap();
    if is_numeric_type(db_type) {
        let v = value.trim();
        if v.eq_ignore_ascii_case("true") {
            return "1".into();
        }
        if v.eq_ignore_ascii_case("false") {
            return "0".into();
        }
        if parses_as_number(v) {
            return v.to_string();
        }
    }
    format!("'{}'", value.replace('\'', "''"))
}

fn re(s: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(s).unwrap())
}

/// Checks that the query is a plain single-table SELECT the grid can safely write back to.
/// Returns a human-readable reason when it is not.
pub fn validate_editable_query(query: &str) -> Option<String> {
    if query.trim().is_empty() {
        return Some("No query text.".into());
    }
    static COMMENTS: OnceLock<Regex> = OnceLock::new();
    static JOIN: OnceLock<Regex> = OnceLock::new();
    static SUBQUERY: OnceLock<Regex> = OnceLock::new();
    static AGG: OnceLock<Regex> = OnceLock::new();
    static SELECT_FROM: OnceLock<Regex> = OnceLock::new();
    static TOP: OnceLock<Regex> = OnceLock::new();
    static AS: OnceLock<Regex> = OnceLock::new();
    static IMPLICIT_ALIAS: OnceLock<Regex> = OnceLock::new();
    static COMPUTED: OnceLock<Regex> = OnceLock::new();

    let stripped = re(r"(?s)--[^\n]*|/\*.*?\*/", &COMMENTS).replace_all(query, " ");
    if re(r"(?i)\bJOIN\b", &JOIN).is_match(&stripped) {
        return Some("the query contains a JOIN".into());
    }
    if re(r"(?i)\(\s*SELECT\b", &SUBQUERY).is_match(&stripped) {
        return Some("the query contains a subquery".into());
    }
    if re(r"(?i)\b(UNION|INTERSECT|EXCEPT|GROUP\s+BY|DISTINCT)\b", &AGG).is_match(&stripped) {
        return Some("the query aggregates or combines rows".into());
    }
    let Some(m) = re(r"(?is)\bSELECT\b(.*?)\bFROM\b", &SELECT_FROM).captures(&stripped) else {
        return Some("the query is not a SELECT ... FROM".into());
    };
    let select_list = m.get(1).map(|g| g.as_str()).unwrap_or("");
    let select_list = re(r"(?i)^\s*TOP\s*(\(\s*\d+\s*\)|\d+)", &TOP).replace(select_list, "");
    if select_list.contains('(') {
        return Some("the select list contains a function or expression".into());
    }
    if re(r"(?i)\s+AS\s+", &AS).is_match(&select_list) || re(r#"[\w\]`"]\s+[\w\[`"]+\s*(,|$)"#, &IMPLICIT_ALIAS).is_match(&select_list) {
        return Some("the select list contains aliased columns".into());
    }
    if re(r"[+/|]|\w\s*[\-*]\s*\w", &COMPUTED).is_match(&select_list) {
        return Some("the select list contains a computed column".into());
    }
    None
}

/// The table in the FROM clause: (schema, table), identifiers unquoted.
pub fn parse_table_ref(query: &str) -> Option<(Option<String>, String)> {
    if query.trim().is_empty() {
        return None;
    }
    static FROM: OnceLock<Regex> = OnceLock::new();
    let m = re(r#"(?i)\bFROM\s+([`\[\"']?\w+[`\]\"']?(?:\s*\.\s*[`\[\"']?\w+[`\]\"']?)?)"#, &FROM).captures(query)?;
    let raw: String = m.get(1)?.as_str().chars().filter(|c| !matches!(c, '[' | ']' | '`' | '"' | '\'')).collect();
    if raw.contains('.') {
        let parts: Vec<&str> = raw.split('.').collect();
        let n = parts.len();
        Some((Some(parts[n - 2].trim().to_string()), parts[n - 1].trim().to_string()))
    } else {
        Some((None, raw.trim().to_string()))
    }
}

pub fn parse_table_name(query: &str) -> Option<String> {
    parse_table_ref(query).map(|(_, t)| t)
}

/// Input for [`generate`]: the result set's columns and the rows before and after editing.
pub struct EditSet<'a> {
    pub table_name: &'a str,
    pub schema: Option<&'a str>,
    pub connection_type: ConnectionType,
    pub pk_columns: &'a HashSet<String>,
    pub column_names: &'a [String],
    pub column_types: &'a [Option<String>],
    pub original_rows: &'a [Vec<Option<String>>],
    pub current_rows: &'a [Vec<Option<String>>],
    pub dirty_rows: &'a [usize],
    pub query_text: &'a str,
}

pub fn generate(e: &EditSet) -> UpdateScript {
    let display_table = match e.schema {
        Some(s) if !s.is_empty() => format!("{s}.{}", e.table_name),
        _ => e.table_name.to_string(),
    };
    if let Some(invalid) = validate_editable_query(e.query_text) {
        return UpdateScript::fail(format!(
            "Cannot apply changes: {invalid}. Edits are only supported on plain single-table SELECT queries."
        ));
    }
    if e.pk_columns.is_empty() {
        return UpdateScript::fail(format!(
            "Cannot apply changes: no primary key found for {display_table}. Edits are only supported on tables with a primary key."
        ));
    }
    let mut where_idx: Vec<usize> = Vec::new();
    let mut pks: Vec<&String> = e.pk_columns.iter().collect();
    pks.sort();
    for pk in pks {
        match e.column_names.iter().position(|c| c.eq_ignore_ascii_case(pk)) {
            Some(i) => where_idx.push(i),
            None => {
                return UpdateScript::fail(format!(
                    "Cannot apply changes: primary key column '{pk}' of {display_table} is not in the result set. Include every primary key column in the SELECT."
                ))
            }
        }
    }
    // WHERE columns in result-set order, so composite keys read naturally.
    where_idx.sort();

    let dialect = dialect_for(e.connection_type);
    let q = |n: &str| quote(dialect, n);
    let type_at = |i: usize| e.column_types.get(i).and_then(|t| t.as_deref());
    let quoted_table = match e.schema {
        Some(s) if !s.is_empty() => format!("{}.{}", q(s), q(e.table_name)),
        _ => q(e.table_name),
    };

    let mut dirty: Vec<usize> = e.dirty_rows.to_vec();
    dirty.sort();
    dirty.dedup();
    let mut statements: Vec<String> = Vec::new();
    for row in dirty {
        if row >= e.original_rows.len() || row >= e.current_rows.len() {
            continue;
        }
        let original = &e.original_rows[row];
        let current = &e.current_rows[row];
        let mut sets: Vec<String> = Vec::new();
        for i in 0..e.column_names.len() {
            let o = original.get(i).cloned().flatten();
            let c = current.get(i).cloned().flatten();
            if o != c {
                sets.push(format!("{} = {}", q(&e.column_names[i]), literal(c.as_deref(), type_at(i))));
            }
        }
        if sets.is_empty() {
            continue;
        }
        let mut wheres: Vec<String> = Vec::new();
        for &i in &where_idx {
            let col = q(&e.column_names[i]);
            let o = original.get(i).cloned().flatten();
            if is_null_value(o.as_deref()) {
                wheres.push(format!("{col} IS NULL"));
            } else {
                wheres.push(format!("{col} = {}", literal(o.as_deref(), type_at(i))));
            }
        }
        statements.push(format!("UPDATE {quoted_table} SET {} WHERE {};", sets.join(", "), wheres.join(" AND ")));
    }
    if statements.is_empty() {
        return UpdateScript::fail("No changes to apply.");
    }

    let mut sql = String::new();
    match dialect {
        SqlDialect::SqlServer => {
            sql.push_str("SET XACT_ABORT ON;\nBEGIN TRAN;\n");
            for st in &statements {
                sql.push_str(st);
                sql.push('\n');
                sql.push_str("IF @@ROWCOUNT <> 1 THROW 50000, 'Expected 1 row', 1;\n");
            }
            sql.push_str("COMMIT;");
        }
        SqlDialect::MySql => {
            sql.push_str("START TRANSACTION;\n");
            for st in &statements {
                sql.push_str(st);
                sql.push('\n');
            }
            sql.push_str("COMMIT;");
        }
        SqlDialect::Sqlite => {
            sql.push_str("BEGIN;\n");
            for st in &statements {
                sql.push_str(st);
                sql.push('\n');
            }
            sql.push_str("COMMIT;");
        }
    }
    UpdateScript { sql, expected_statements: statements.len(), error: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<String> {
        ["id", "name", "qty"].iter().map(|s| s.to_string()).collect()
    }
    fn types() -> Vec<Option<String>> {
        vec![Some("int".into()), Some("nvarchar".into()), Some("decimal".into())]
    }
    fn rows(r: &[&[Option<&str>]]) -> Vec<Vec<Option<String>>> {
        r.iter().map(|row| row.iter().map(|c| c.map(|s| s.to_string())).collect()).collect()
    }
    fn pk(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }
    fn gen(kind: ConnectionType, orig: &[Vec<Option<String>>], cur: &[Vec<Option<String>>], dirty: &[usize], query: &str, pks: &HashSet<String>, schema: Option<&str>) -> UpdateScript {
        let c = cols();
        let t = types();
        generate(&EditSet {
            table_name: "people",
            schema,
            connection_type: kind,
            pk_columns: pks,
            column_names: &c,
            column_types: &t,
            original_rows: orig,
            current_rows: cur,
            dirty_rows: dirty,
            query_text: query,
        })
    }
    const Q: &str = "SELECT id, name, qty FROM people";

    #[test]
    fn no_primary_key_is_error() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        let s = gen(ConnectionType::SqlServer, &o, &c, &[0], Q, &HashSet::new(), None);
        assert!(s.is_error());
        assert!(s.error.unwrap().to_lowercase().contains("primary key"));
    }

    #[test]
    fn pk_missing_from_select_is_error() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        let s = gen(ConnectionType::SqlServer, &o, &c, &[0], Q, &pk(&["other_id"]), None);
        assert!(s.error.unwrap().contains("other_id"));
    }

    #[test]
    fn unsafe_queries_are_rejected() {
        let cases = [
            ("SELECT p.id, p.name, p.qty FROM people p JOIN other o ON o.id = p.id", "join"),
            ("SELECT id, name AS n, qty FROM people", "alias"),
            ("SELECT id, name n, qty FROM people", "alias"),
            ("SELECT id, name, qty FROM people WHERE id IN (SELECT id FROM x)", "subquery"),
            ("SELECT DISTINCT id, name, qty FROM people", "aggregates"),
            ("SELECT id, UPPER(name), qty FROM people", "function"),
            ("SELECT id, name, qty * 2 FROM people", "computed"),
            ("UPDATE people SET name = 'x'", "not a select"),
            ("", "no query"),
        ];
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        for (q, reason) in cases {
            let s = gen(ConnectionType::SqlServer, &o, &c, &[0], q, &pk(&["id"]), None);
            assert!(s.is_error(), "{q}");
            assert!(s.error.unwrap().to_lowercase().contains(reason), "{q}");
        }
    }

    #[test]
    fn comments_are_ignored_by_validation() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        let s = gen(ConnectionType::SqlServer, &o, &c, &[0], "-- JOIN in a comment\nSELECT id, name, qty /* AS x */ FROM people", &pk(&["id"]), None);
        assert!(!s.is_error(), "{:?}", s.error);
    }

    #[test]
    fn sql_server_happy_path() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")], &[Some("2"), Some("b"), Some("2")], &[Some("3"), Some("c"), Some("3")]]);
        let c = rows(&[&[Some("1"), Some("O'Brien"), Some("1")], &[Some("2"), Some("b"), Some("2.5")], &[Some("3"), None, Some("3")]]);
        let s = gen(ConnectionType::SqlServer, &o, &c, &[2, 0, 1], Q, &pk(&["id"]), None);
        assert!(!s.is_error(), "{:?}", s.error);
        assert_eq!(s.expected_statements, 3);
        assert!(s.sql.starts_with("SET XACT_ABORT ON;"));
        assert!(s.sql.contains("BEGIN TRAN;"));
        assert!(s.sql.ends_with("COMMIT;"));
        assert_eq!(s.sql.matches("IF @@ROWCOUNT <> 1 THROW 50000, 'Expected 1 row', 1;").count(), 3);
        assert!(s.sql.contains("UPDATE [people] SET [name] = 'O''Brien' WHERE [id] = 1;"));
        assert!(s.sql.contains("UPDATE [people] SET [qty] = 2.5 WHERE [id] = 2;"));
        assert!(s.sql.contains("UPDATE [people] SET [name] = NULL WHERE [id] = 3;"));
        assert!(!s.sql.contains("[name] = 'a'"));
        assert!(s.sql.find("WHERE [id] = 1") < s.sql.find("WHERE [id] = 2"));
    }

    #[test]
    fn schema_is_quoted_separately() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        let s = gen(ConnectionType::SqlServer, &o, &c, &[0], Q, &pk(&["id"]), Some("dbo"));
        assert!(s.sql.contains("UPDATE [dbo].[people] SET"));
    }

    #[test]
    fn mysql_wrapper_and_backticks() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        let s = gen(ConnectionType::MySql, &o, &c, &[0], Q, &pk(&["id"]), None);
        assert_eq!(s.sql, "START TRANSACTION;\nUPDATE `people` SET `name` = 'b' WHERE `id` = 1;\nCOMMIT;");
    }

    #[test]
    fn sqlite_wrapper_and_double_quotes() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        let s = gen(ConnectionType::Sqlite, &o, &c, &[0], Q, &pk(&["id"]), None);
        assert_eq!(s.sql, "BEGIN;\nUPDATE \"people\" SET \"name\" = 'b' WHERE \"id\" = 1;\nCOMMIT;");
    }

    #[test]
    fn null_pk_uses_is_null() {
        let o = rows(&[&[None, Some("a"), Some("1")]]);
        let c = rows(&[&[None, Some("b"), Some("1")]]);
        assert!(gen(ConnectionType::SqlServer, &o, &c, &[0], Q, &pk(&["id"]), None).sql.contains("WHERE [id] IS NULL;"));
        let o = rows(&[&[Some("null"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("null"), Some("b"), Some("1")]]);
        assert!(gen(ConnectionType::SqlServer, &o, &c, &[0], Q, &pk(&["id"]), None).sql.contains("WHERE [id] IS NULL;"));
    }

    #[test]
    fn unchanged_rows_are_skipped_and_none_is_error() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")], &[Some("2"), Some("b"), Some("2")]]);
        let s = gen(ConnectionType::SqlServer, &o, &o, &[0, 1], Q, &pk(&["id"]), None);
        assert_eq!(s.error.as_deref(), Some("No changes to apply."));
    }

    #[test]
    fn out_of_range_dirty_indices_are_ignored() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("b"), Some("1")]]);
        let s = gen(ConnectionType::SqlServer, &o, &c, &[0, 7], Q, &pk(&["id"]), None);
        assert_eq!(s.expected_statements, 1);
    }

    #[test]
    fn composite_pk_all_columns_in_where() {
        let o = rows(&[&[Some("1"), Some("a"), Some("1")]]);
        let c = rows(&[&[Some("1"), Some("a"), Some("9")]]);
        let s = gen(ConnectionType::SqlServer, &o, &c, &[0], Q, &pk(&["ID", "NAME"]), None);
        assert!(s.sql.contains("SET [qty] = 9 WHERE [id] = 1 AND [name] = 'a';"), "{}", s.sql);
    }

    #[test]
    fn literal_formats() {
        let cases: [(Option<&str>, Option<&str>, &str); 10] = [
            (None, Some("int"), "NULL"),
            (Some("null"), Some("int"), "NULL"),
            (Some("42"), Some("int"), "42"),
            (Some(" 42 "), Some("bigint"), "42"),
            (Some("1e3"), Some("float"), "1e3"),
            (Some("true"), Some("bit"), "1"),
            (Some("FALSE"), Some("bit"), "0"),
            (Some("abc"), Some("int"), "'abc'"),
            (Some("42"), Some("varchar"), "'42'"),
            (Some("it's"), None, "'it''s'"),
        ];
        for (v, t, expected) in cases {
            assert_eq!(literal(v, t), expected);
        }
    }

    #[test]
    fn parse_table_ref_cases() {
        assert_eq!(parse_table_ref("SELECT * FROM dbo.People p"), Some((Some("dbo".into()), "People".into())));
        assert_eq!(parse_table_ref("select * from [dbo].[People]"), Some((Some("dbo".into()), "People".into())));
        assert_eq!(parse_table_ref("SELECT * FROM `t`"), Some((None, "t".into())));
        assert_eq!(parse_table_ref("SELECT 1"), None);
    }
}
