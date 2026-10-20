//! Completion keywords: a common set plus dialect-specific functions and types.

use crate::models::ConnectionType;

pub fn for_dialect(kind: ConnectionType) -> Vec<String> {
    let mut list: Vec<String> = COMMON.iter().map(|s| s.to_string()).collect();
    let extra: &[&str] = match kind {
        ConnectionType::SqlServer => SQL_SERVER,
        ConnectionType::MySql => MYSQL,
        ConnectionType::Sqlite => SQLITE,
    };
    list.extend(extra.iter().map(|s| s.to_string()));
    list
}

const COMMON: &[&str] = &[
    // DML
    "SELECT", "FROM", "WHERE", "JOIN", "INNER JOIN", "LEFT JOIN", "RIGHT JOIN", "CROSS JOIN", "FULL JOIN", "ON", "AND", "OR",
    "NOT", "IN", "BETWEEN", "LIKE", "IS", "NULL", "IS NOT NULL", "EXISTS", "INSERT INTO", "VALUES", "UPDATE", "SET", "DELETE",
    "ORDER BY", "GROUP BY", "HAVING", "AS", "DISTINCT", "UNION", "UNION ALL", "EXCEPT", "INTERSECT", "ASC", "DESC", "LIMIT",
    "OFFSET",
    // DDL
    "CREATE", "ALTER", "DROP", "TABLE", "INDEX", "VIEW", "PRIMARY KEY", "FOREIGN KEY", "REFERENCES", "UNIQUE", "DEFAULT",
    "CONSTRAINT", "CASCADE", "TRUNCATE",
    // Control
    "CASE", "WHEN", "THEN", "ELSE", "END", "BEGIN", "COMMIT", "ROLLBACK",
    // Aggregate
    "COUNT", "SUM", "AVG", "MIN", "MAX",
    // Common functions
    "COALESCE", "CAST", "UPPER", "LOWER", "TRIM", "LTRIM", "RTRIM", "SUBSTRING", "REPLACE", "CONCAT", "LENGTH", "LEN", "ABS",
    "CEILING", "FLOOR", "ROUND", "YEAR", "MONTH", "DAY",
];

const SQL_SERVER: &[&str] = &[
    "TOP", "FETCH", "NEXT", "ROWS", "ONLY", "OUTPUT", "MERGE", "USING", "MATCHED", "PROCEDURE", "FUNCTION", "TRIGGER", "EXEC",
    "EXECUTE", "DECLARE", "GO", "IDENTITY", "NEWID", "SCOPE_IDENTITY", "GETDATE()", "GETUTCDATE()", "SYSDATETIME()",
    "SYSUTCDATETIME()", "DATEADD", "DATEDIFF", "DATEPART", "DATENAME", "EOMONTH", "ISNULL", "NULLIF", "IIF", "CONVERT",
    "TRY_CONVERT", "TRY_CAST", "FORMAT", "CHARINDEX", "PATINDEX", "STUFF", "STRING_AGG", "LEFT", "RIGHT", "ROW_NUMBER()",
    "RANK()", "DENSE_RANK()", "NTILE", "LAG", "LEAD", "FIRST_VALUE", "LAST_VALUE", "OVER", "PARTITION BY", "ISNUMERIC", "ISDATE",
    "OBJECT_ID", "DB_NAME()", "SCHEMA_NAME", "@@ROWCOUNT", "@@IDENTITY", "@@ERROR", "NVARCHAR", "VARCHAR", "INT", "BIGINT",
    "BIT", "DATETIME", "DATETIME2", "DECIMAL", "FLOAT", "MONEY", "UNIQUEIDENTIFIER", "XML",
];

const MYSQL: &[&str] = &[
    "PROCEDURE", "FUNCTION", "TRIGGER", "DELIMITER", "CALL", "AUTO_INCREMENT", "ENGINE", "CHARSET", "COLLATE", "IF EXISTS",
    "IF NOT EXISTS", "SHOW DATABASES", "SHOW TABLES", "SHOW COLUMNS", "DESCRIBE", "USE", "EXPLAIN", "NOW()", "CURDATE()",
    "CURTIME()", "CURRENT_TIMESTAMP()", "SYSDATE()", "UTC_DATE()", "UTC_TIME()", "UTC_TIMESTAMP()", "DATE_ADD", "DATE_SUB",
    "DATEDIFF", "TIMESTAMPDIFF", "TIMEDIFF", "DATE_FORMAT", "STR_TO_DATE", "EXTRACT", "IFNULL", "NULLIF", "IF", "CONCAT_WS",
    "GROUP_CONCAT", "FIND_IN_SET", "LPAD", "RPAD", "REVERSE", "CHAR_LENGTH", "CHARACTER_LENGTH", "INET_ATON", "INET_NTOA",
    "JSON_EXTRACT", "JSON_UNQUOTE", "JSON_SET", "JSON_OBJECT", "JSON_ARRAY", "UUID()", "LAST_INSERT_ID()", "ROW_COUNT()",
    "FOUND_ROWS()", "CONNECTION_ID()", "DATABASE()", "USER()", "VERSION()", "MD5", "SHA1", "SHA2", "REGEXP", "RLIKE",
    "SOUNDS LIKE", "CONVERT", "BINARY", "INT", "BIGINT", "TINYINT", "SMALLINT", "MEDIUMINT", "VARCHAR", "CHAR", "TEXT",
    "MEDIUMTEXT", "LONGTEXT", "DATETIME", "TIMESTAMP", "DATE", "TIME", "DECIMAL", "FLOAT", "DOUBLE", "BOOLEAN", "BLOB",
    "MEDIUMBLOB", "LONGBLOB", "JSON", "ENUM", "SET",
];

const SQLITE: &[&str] = &[
    "AUTOINCREMENT", "ROWID", "WITHOUT ROWID", "IF EXISTS", "IF NOT EXISTS", "PRAGMA", "VACUUM", "ATTACH", "DETACH", "EXPLAIN",
    "EXPLAIN QUERY PLAN", "GLOB", "REGEXP", "datetime('now')", "date('now')", "time('now')", "strftime", "julianday", "typeof",
    "total", "group_concat", "ifnull", "nullif", "iif", "instr", "unicode", "char", "hex", "zeroblob", "randomblob", "random()",
    "last_insert_rowid()", "changes()", "total_changes()", "sqlite_version()", "sqlite_source_id()", "json", "json_extract",
    "json_insert", "json_replace", "json_set", "json_object", "json_array", "json_type", "json_valid", "json_group_array",
    "json_group_object", "printf", "format", "likely", "unlikely", "INTEGER", "TEXT", "REAL", "BLOB", "NUMERIC",
];

/// Schema that unqualified table names resolve to for the dialect.
pub fn default_schema(kind: ConnectionType) -> Option<String> {
    match kind {
        ConnectionType::SqlServer => Some("dbo".into()),
        _ => None,
    }
}
