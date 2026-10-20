use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SqlDialect {
    SqlServer,
    MySql,
    Sqlite,
}

/// Quote an identifier for interpolation into SQL, escaping the closing delimiter.
pub fn quote(dialect: SqlDialect, name: &str) -> String {
    match dialect {
        SqlDialect::SqlServer => format!("[{}]", name.replace(']', "]]")),
        SqlDialect::MySql => format!("`{}`", name.replace('`', "``")),
        SqlDialect::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// `schema.name` with both parts quoted, or just the name when there is no schema.
pub fn qualified(dialect: SqlDialect, schema: &str, name: &str) -> String {
    if schema.is_empty() {
        quote(dialect, name)
    } else {
        format!("{}.{}", quote(dialect, schema), quote(dialect, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_server_uses_brackets_and_doubles_closing() {
        assert_eq!(quote(SqlDialect::SqlServer, "col"), "[col]");
        assert_eq!(quote(SqlDialect::SqlServer, "a]b"), "[a]]b]");
        assert_eq!(quote(SqlDialect::SqlServer, "a[b"), "[a[b]");
    }

    #[test]
    fn mysql_uses_backticks() {
        assert_eq!(quote(SqlDialect::MySql, "col"), "`col`");
        assert_eq!(quote(SqlDialect::MySql, "a`b"), "`a``b`");
    }

    #[test]
    fn sqlite_uses_double_quotes() {
        assert_eq!(quote(SqlDialect::Sqlite, "col"), "\"col\"");
        assert_eq!(quote(SqlDialect::Sqlite, "a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn qualified_names() {
        assert_eq!(qualified(SqlDialect::SqlServer, "dbo", "People"), "[dbo].[People]");
        assert_eq!(qualified(SqlDialect::MySql, "", "t"), "`t`");
    }
}
