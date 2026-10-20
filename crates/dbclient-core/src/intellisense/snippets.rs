//! Typing shortcuts in the SQL Prompt style: `ssf` → `SELECT * FROM `. `$0` marks where the
//! caret ends up; `${1:text}` a placeholder the editor can select.

use crate::SqlDialect;

pub struct Snippet {
    pub shortcut: &'static str,
    pub label: &'static str,
    pub body: &'static str,
}

const COMMON: &[Snippet] = &[
    Snippet { shortcut: "ssf", label: "SELECT * FROM", body: "SELECT * FROM $0" },
    Snippet { shortcut: "ssc", label: "SELECT COUNT(*) FROM", body: "SELECT COUNT(*) FROM $0" },
    Snippet { shortcut: "sd", label: "SELECT DISTINCT ... FROM", body: "SELECT DISTINCT ${1:column} FROM $0" },
    Snippet { shortcut: "ii", label: "INSERT INTO ... VALUES", body: "INSERT INTO ${1:table} (${2:columns})\nVALUES (${3:values})$0" },
    Snippet { shortcut: "iis", label: "INSERT INTO ... SELECT", body: "INSERT INTO ${1:table} (${2:columns})\nSELECT ${3:columns}\nFROM $0" },
    Snippet { shortcut: "uu", label: "UPDATE ... SET ... WHERE", body: "UPDATE ${1:table}\nSET ${2:column} = ${3:value}\nWHERE $0" },
    Snippet { shortcut: "dd", label: "DELETE FROM ... WHERE", body: "DELETE FROM ${1:table}\nWHERE $0" },
    Snippet { shortcut: "cte", label: "WITH cte AS (...)", body: "WITH ${1:cte} AS (\n    SELECT $0\n)\nSELECT *\nFROM ${1:cte}" },
    Snippet { shortcut: "ij", label: "INNER JOIN ... ON", body: "INNER JOIN ${1:table} ON $0" },
    Snippet { shortcut: "lj", label: "LEFT JOIN ... ON", body: "LEFT JOIN ${1:table} ON $0" },
    Snippet { shortcut: "ob", label: "ORDER BY", body: "ORDER BY $0" },
    Snippet { shortcut: "gb", label: "GROUP BY", body: "GROUP BY $0" },
    Snippet { shortcut: "cs", label: "CASE WHEN ... THEN ... ELSE ... END", body: "CASE WHEN ${1:condition} THEN ${2:value} ELSE ${3:value} END$0" },
    Snippet { shortcut: "ex", label: "EXISTS (SELECT 1 FROM ...)", body: "EXISTS (SELECT 1 FROM ${1:table} WHERE $0)" },
    Snippet { shortcut: "ct", label: "CREATE TABLE", body: "CREATE TABLE ${1:table} (\n    ${2:Id} INT NOT NULL PRIMARY KEY,\n    $0\n)" },
];

const SQL_SERVER: &[Snippet] = &[
    Snippet { shortcut: "st", label: "SELECT TOP 100 * FROM", body: "SELECT TOP 100 * FROM $0" },
    Snippet { shortcut: "st1", label: "SELECT TOP 1 * FROM", body: "SELECT TOP 1 * FROM $0" },
    Snippet { shortcut: "bt", label: "BEGIN TRAN ... COMMIT", body: "BEGIN TRAN\n$0\nCOMMIT" },
    Snippet { shortcut: "tc", label: "BEGIN TRY ... END TRY BEGIN CATCH ... END CATCH", body: "BEGIN TRY\n    $0\nEND TRY\nBEGIN CATCH\n    THROW\nEND CATCH" },
    Snippet { shortcut: "dv", label: "DECLARE variable", body: "DECLARE @${1:name} ${2:INT} = $0" },
    Snippet { shortcut: "iif", label: "IF EXISTS (SELECT ...)", body: "IF EXISTS (SELECT 1 FROM ${1:table} WHERE ${2:condition})\nBEGIN\n    $0\nEND" },
    Snippet { shortcut: "sp", label: "CREATE PROCEDURE", body: "CREATE PROCEDURE ${1:dbo.ProcName}\n    @${2:param} INT\nAS\nBEGIN\n    SET NOCOUNT ON;\n    $0\nEND" },
    Snippet { shortcut: "mg", label: "MERGE ... USING ... ON", body: "MERGE ${1:target} AS t\nUSING ${2:source} AS s ON t.${3:Id} = s.${3:Id}\nWHEN MATCHED THEN UPDATE SET $0\nWHEN NOT MATCHED THEN INSERT () VALUES ();" },
];

const MYSQL: &[Snippet] = &[
    Snippet { shortcut: "sl", label: "SELECT * FROM ... LIMIT 100", body: "SELECT * FROM ${1:table} LIMIT 100$0" },
    Snippet { shortcut: "bt", label: "START TRANSACTION ... COMMIT", body: "START TRANSACTION;\n$0\nCOMMIT;" },
    Snippet { shortcut: "odk", label: "INSERT ... ON DUPLICATE KEY UPDATE", body: "INSERT INTO ${1:table} (${2:columns}) VALUES (${3:values})\nON DUPLICATE KEY UPDATE $0" },
];

const SQLITE: &[Snippet] = &[
    Snippet { shortcut: "sl", label: "SELECT * FROM ... LIMIT 100", body: "SELECT * FROM ${1:table} LIMIT 100$0" },
    Snippet { shortcut: "bt", label: "BEGIN ... COMMIT", body: "BEGIN;\n$0\nCOMMIT;" },
    Snippet { shortcut: "pti", label: "PRAGMA table_info", body: "PRAGMA table_info(${1:table})$0" },
];

pub fn for_dialect(dialect: SqlDialect) -> impl Iterator<Item = &'static Snippet> {
    let extra: &'static [Snippet] = match dialect {
        SqlDialect::SqlServer => SQL_SERVER,
        SqlDialect::MySql => MYSQL,
        SqlDialect::Sqlite => SQLITE,
    };
    COMMON.iter().chain(extra.iter())
}

/// Strip placeholder markup for a plain-text preview.
pub fn plain(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek() {
                Some('0') => {
                    chars.next();
                }
                Some('{') => {
                    chars.next();
                    let mut inner = String::new();
                    for d in chars.by_ref() {
                        if d == '}' {
                            break;
                        }
                        inner.push(d);
                    }
                    if let Some((_, text)) = inner.split_once(':') {
                        out.push_str(text);
                    }
                }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_strips_markup() {
        assert_eq!(plain("UPDATE ${1:table}\nSET ${2:column} = ${3:value}\nWHERE $0"), "UPDATE table\nSET column = value\nWHERE ");
    }

    #[test]
    fn shortcuts_are_unique_per_dialect() {
        for d in [SqlDialect::SqlServer, SqlDialect::MySql, SqlDialect::Sqlite] {
            let mut seen = std::collections::HashSet::new();
            for s in for_dialect(d) {
                assert!(seen.insert(s.shortcut), "duplicate {}", s.shortcut);
            }
        }
    }
}
