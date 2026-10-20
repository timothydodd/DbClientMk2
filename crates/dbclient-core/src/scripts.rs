//! SQL templates the schema tree and menus insert: SELECT/INSERT/UPDATE/DELETE skeletons,
//! CREATE/DROP scripts, EXPLAIN wrappers, database rename/drop.

use crate::models::{ConnectionType, DbColumn};
use crate::sql_identifier::{qualified, quote, SqlDialect};

pub fn select_all(dialect: SqlDialect, schema: &str, name: &str) -> String {
    format!("SELECT * FROM {}", qualified(dialect, schema, name))
}

pub fn select_top(kind: ConnectionType, schema: &str, name: &str, n: u32) -> String {
    let dialect = crate::update_sql::dialect_for(kind);
    let q = qualified(dialect, schema, name);
    match kind {
        ConnectionType::SqlServer => format!("SELECT TOP {n} * FROM {q}"),
        _ => format!("SELECT * FROM {q} LIMIT {n}"),
    }
}

/// Double-click on a table: like [`select_top`] but with SQL Server's `TOP(100)` form.
pub fn select_top_paren(kind: ConnectionType, schema: &str, name: &str, n: u32) -> String {
    let dialect = crate::update_sql::dialect_for(kind);
    let q = qualified(dialect, schema, name);
    match kind {
        ConnectionType::SqlServer => format!("SELECT TOP({n}) * FROM {q}"),
        _ => format!("SELECT * FROM {q} LIMIT {n}"),
    }
}

pub fn select_count(dialect: SqlDialect, schema: &str, name: &str) -> String {
    format!("SELECT COUNT(*) FROM {}", qualified(dialect, schema, name))
}

pub fn insert_into(dialect: SqlDialect, schema: &str, name: &str) -> String {
    format!("INSERT INTO {} ()\nVALUES ()", qualified(dialect, schema, name))
}

pub fn update(dialect: SqlDialect, schema: &str, name: &str) -> String {
    format!("UPDATE {}\nSET \nWHERE ", qualified(dialect, schema, name))
}

pub fn delete_from(dialect: SqlDialect, schema: &str, name: &str) -> String {
    format!("DELETE FROM {}\nWHERE ", qualified(dialect, schema, name))
}

pub fn drop_table(dialect: SqlDialect, schema: &str, name: &str) -> String {
    format!("DROP TABLE {}", qualified(dialect, schema, name))
}

pub fn drop_view(dialect: SqlDialect, schema: &str, name: &str) -> String {
    format!("DROP VIEW {}", qualified(dialect, schema, name))
}

pub fn exec_proc(kind: ConnectionType, schema: &str, name: &str) -> String {
    let q = qualified(crate::update_sql::dialect_for(kind), schema, name);
    match kind {
        ConnectionType::MySql => format!("CALL {q}()"),
        _ => format!("EXEC {q}"),
    }
}

pub fn create_table(dialect: SqlDialect, schema: &str, name: &str, columns: &[DbColumn]) -> String {
    let mut out = format!("CREATE TABLE {} (\n", qualified(dialect, schema, name));
    let pks: Vec<&DbColumn> = columns.iter().filter(|c| c.is_primary_key).collect();
    for (i, col) in columns.iter().enumerate() {
        let is_pk = col.is_primary_key;
        let nullable = col.is_nullable && !is_pk;
        out.push_str(&format!("    {} {}", quote(dialect, &col.name), col.data_type));
        if !nullable && !is_pk {
            out.push_str(" NOT NULL");
        }
        if i < columns.len() - 1 || !pks.is_empty() {
            out.push(',');
        }
        out.push('\n');
    }
    if !pks.is_empty() {
        out.push_str(&format!("    PRIMARY KEY ({})\n", pks.iter().map(|c| quote(dialect, &c.name)).collect::<Vec<_>>().join(", ")));
    }
    out.push_str(");");
    out
}

pub fn explain(kind: ConnectionType, query: &str) -> String {
    match kind {
        ConnectionType::Sqlite => format!("EXPLAIN QUERY PLAN {query}"),
        ConnectionType::SqlServer => format!("SET SHOWPLAN_TEXT ON;\n{query}\nSET SHOWPLAN_TEXT OFF;"),
        ConnectionType::MySql => format!("EXPLAIN {query}"),
    }
}

/// Server-level database used for administrative statements.
pub fn admin_database(kind: ConnectionType) -> &'static str {
    match kind {
        ConnectionType::MySql => "information_schema",
        _ => "master",
    }
}

pub fn supports_rename_database(kind: ConnectionType) -> bool {
    kind == ConnectionType::SqlServer
}

pub fn supports_drop_database(kind: ConnectionType) -> bool {
    matches!(kind, ConnectionType::SqlServer | ConnectionType::MySql)
}

/// SQL Server only: kick other sessions, rename, reopen.
pub fn rename_database(kind: ConnectionType, old: &str, new: &str) -> Option<String> {
    if !supports_rename_database(kind) {
        return None;
    }
    let d = SqlDialect::SqlServer;
    Some(format!(
        "ALTER DATABASE {o} SET SINGLE_USER WITH ROLLBACK IMMEDIATE;\nALTER DATABASE {o} MODIFY NAME = {n};\nALTER DATABASE {n} SET MULTI_USER;",
        o = quote(d, old),
        n = quote(d, new)
    ))
}

pub fn drop_database(kind: ConnectionType, name: &str) -> Option<String> {
    match kind {
        ConnectionType::SqlServer => {
            let q = quote(SqlDialect::SqlServer, name);
            Some(format!("ALTER DATABASE {q} SET SINGLE_USER WITH ROLLBACK IMMEDIATE;\nDROP DATABASE {q};"))
        }
        ConnectionType::MySql => Some(format!("DROP DATABASE {};", quote(SqlDialect::MySql, name))),
        ConnectionType::Sqlite => None,
    }
}

/// Appends `sql` to existing editor text with a blank line between statements.
pub fn append_statement(existing: &str, sql: &str) -> String {
    if existing.trim().is_empty() {
        return sql.to_string();
    }
    let sep = if existing.ends_with('\n') { "\n" } else { "\n\n" };
    format!("{}{}{}", existing.trim_end(), sep, sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_forms() {
        assert_eq!(select_top(ConnectionType::SqlServer, "dbo", "T", 100), "SELECT TOP 100 * FROM [dbo].[T]");
        assert_eq!(select_top(ConnectionType::MySql, "", "t", 100), "SELECT * FROM `t` LIMIT 100");
        assert_eq!(select_top_paren(ConnectionType::SqlServer, "", "T", 100), "SELECT TOP(100) * FROM [T]");
    }

    #[test]
    fn create_table_script() {
        let cols = vec![
            DbColumn { name: "id".into(), data_type: "int".into(), is_primary_key: true, is_nullable: false },
            DbColumn { name: "name".into(), data_type: "varchar(50)".into(), is_primary_key: false, is_nullable: true },
        ];
        let s = create_table(SqlDialect::SqlServer, "", "t", &cols);
        assert_eq!(s, "CREATE TABLE [t] (\n    [id] int,\n    [name] varchar(50),\n    PRIMARY KEY ([id])\n);");
    }

    #[test]
    fn rename_and_drop() {
        assert!(rename_database(ConnectionType::MySql, "a", "b").is_none());
        assert!(rename_database(ConnectionType::SqlServer, "a", "b").unwrap().contains("MODIFY NAME = [b]"));
        assert_eq!(drop_database(ConnectionType::MySql, "a").unwrap(), "DROP DATABASE `a`;");
        assert!(drop_database(ConnectionType::Sqlite, "a").is_none());
    }

    #[test]
    fn append_keeps_a_blank_line() {
        assert_eq!(append_statement("", "SELECT 1"), "SELECT 1");
        assert_eq!(append_statement("SELECT 0", "SELECT 1"), "SELECT 0\n\nSELECT 1");
        assert_eq!(append_statement("SELECT 0\n", "SELECT 1"), "SELECT 0\nSELECT 1");
    }
}
