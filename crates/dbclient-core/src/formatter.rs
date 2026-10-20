//! A small SQL pretty-printer: clause keywords on their own lines, SELECT list items one per
//! line, AND/OR indented. String literals and comments pass through untouched.

const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "JOIN", "INNER", "LEFT", "RIGHT", "CROSS", "FULL", "OUTER", "ON", "AND", "OR", "GROUP", "BY",
    "ORDER", "HAVING", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "ALTER", "DROP", "TABLE", "VIEW", "INDEX",
    "UNION", "ALL", "AS", "CASE", "WHEN", "THEN", "ELSE", "END", "IN", "NOT", "NULL", "IS", "LIKE", "BETWEEN", "EXISTS", "DISTINCT",
    "TOP", "LIMIT", "OFFSET", "WITH", "EXEC", "CALL", "BEGIN", "COMMIT", "ROLLBACK", "ASC", "DESC", "COUNT", "SUM", "AVG", "MIN",
    "MAX", "COALESCE", "CAST", "CONVERT", "IF", "IFNULL", "ISNULL", "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "CONSTRAINT",
    "DEFAULT", "CHECK", "UNIQUE", "TRUNCATE", "DECLARE", "RETURNS", "PROCEDURE", "FUNCTION",
];

const NEWLINE_BEFORE: &[&str] = &[
    "FROM", "WHERE", "JOIN", "INNER", "LEFT", "RIGHT", "CROSS", "FULL", "GROUP", "ORDER", "HAVING", "UNION", "SET", "VALUES",
    "LIMIT", "OFFSET",
];

const INDENT_KEYWORDS: &[&str] = &["AND", "OR"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Symbol,
    StringLiteral,
    Comment,
    Whitespace,
}

struct Token {
    value: String,
    kind: Kind,
}

fn is_in(set: &[&str], token: &str) -> bool {
    set.iter().any(|k| k.eq_ignore_ascii_case(token))
}

pub fn format(sql: &str) -> String {
    if sql.trim().is_empty() {
        return sql.to_string();
    }
    let tokens = tokenize(sql);
    let mut out = String::new();
    let indent = "    ";
    let mut after_select = false;
    let mut select_depth = 0;

    for token in &tokens {
        match token.kind {
            Kind::StringLiteral | Kind::Comment => {
                if matches!(out.chars().last(), Some(c) if c != '\n' && c != ' ' && c != '(') {
                    out.push(' ');
                }
                out.push_str(&token.value);
                continue;
            }
            Kind::Whitespace => continue,
            _ => {}
        }
        let upper = token.value.to_uppercase();
        let is_keyword = is_in(KEYWORDS, &token.value);

        if upper == "SELECT" {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("SELECT");
            after_select = true;
            select_depth = 0;
            continue;
        }
        if (upper == "DISTINCT" || upper == "TOP") && after_select {
            out.push(' ');
            out.push_str(&upper);
            continue;
        }
        if is_in(NEWLINE_BEFORE, &token.value) {
            after_select = false;
            out.push('\n');
            out.push_str(if is_keyword { &upper } else { &token.value });
            continue;
        }
        if is_in(INDENT_KEYWORDS, &token.value) {
            after_select = false;
            out.push('\n');
            out.push_str(indent);
            out.push_str(&upper);
            continue;
        }
        if token.value == "," && after_select {
            out.push(',');
            out.push('\n');
            out.push_str(indent);
            continue;
        }
        if after_select && select_depth == 0 {
            out.push(' ');
            select_depth += 1;
        } else if let Some(last) = out.chars().last() {
            if last != '\n' && last != ' ' && last != '(' && token.value != ")" && token.value != "," {
                out.push(' ');
            }
        }
        out.push_str(if is_keyword { &upper } else { &token.value });
    }
    out.trim().to_string()
}

fn tokenize(sql: &str) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let push = |tokens: &mut Vec<Token>, s: &[char], kind: Kind| tokens.push(Token { value: s.iter().collect(), kind });

    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' && i + 1 < chars.len() && chars[i + 1] == '\'' {
                    i += 2;
                } else if chars[i] == '\'' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            push(&mut tokens, &chars[start..i], Kind::StringLiteral);
            continue;
        }
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            push(&mut tokens, &chars[start..i], Kind::Comment);
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2;
            } else {
                i = chars.len();
            }
            push(&mut tokens, &chars[start..i], Kind::Comment);
            continue;
        }
        if c.is_whitespace() {
            let start = i;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            push(&mut tokens, &chars[start..i], Kind::Whitespace);
            continue;
        }
        if c.is_alphanumeric() || c == '_' || c == '@' || c == '#' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '@' | '#' | '.')) {
                i += 1;
            }
            push(&mut tokens, &chars[start..i], Kind::Word);
            continue;
        }
        if c == '[' || c == '`' || c == '"' {
            let start = i;
            let closer = if c == '[' { ']' } else { c };
            i += 1;
            while i < chars.len() && chars[i] != closer {
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            push(&mut tokens, &chars[start..i], Kind::Word);
            continue;
        }
        push(&mut tokens, &chars[i..i + 1], Kind::Symbol);
        i += 1;
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_simple_select() {
        let out = format("select a, b from t where a = 1 and b = 'x y' order by a");
        assert_eq!(out, "SELECT a,\n    b\nFROM t\nWHERE a = 1\n    AND b = 'x y'\nORDER BY a");
    }

    #[test]
    fn keeps_comments_and_strings() {
        let out = format("select 'it''s' -- note\nfrom t");
        assert!(out.contains("'it''s'"));
        assert!(out.contains("-- note"));
    }

    #[test]
    fn blank_input_is_returned_as_is() {
        assert_eq!(format("   "), "   ");
    }
}
