//! Classifies the caret position using the statement structure from [`scope`]: which block and
//! clause it is in, what is being typed, what sources are visible. Aliases come from the whole
//! statement (including text after the caret), so `SELECT | FROM Users u` offers `u`'s columns.

use super::lexer::TokenKind;
use super::scope::{self, Analysis, BlockKind, ClauseKind, SourceKind};
use super::{Aliases, SqlContext, SqlContextType};

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The identifier characters immediately before the caret (also `@var` / `#tmp` prefixes).
fn partial_word(text: &[char], caret: usize) -> (String, usize) {
    let mut start = caret;
    while start > 0 && is_word_char(text[start - 1]) {
        start -= 1;
    }
    while start > 0 && (text[start - 1] == '@' || text[start - 1] == '#') {
        start -= 1;
    }
    (text[start..caret].iter().collect(), start)
}

/// `dbo.Orders.` → `dbo.Orders` for a dot right before `pos` (whitespace allowed between).
fn qualifier_before(text: &[char], mut pos: usize) -> Option<String> {
    while pos > 0 && text[pos - 1].is_whitespace() {
        pos -= 1;
    }
    if pos == 0 || text[pos - 1] != '.' {
        return None;
    }
    let mut i = pos as isize - 2;
    let mut out: Vec<char> = Vec::new();
    while i >= 0 {
        let c = text[i as usize];
        if is_word_char(c) || c == '@' || c == '#' {
            out.insert(0, c);
            i -= 1;
        } else if c == ']' || c == '`' || c == '"' {
            let open = if c == ']' { '[' } else { c };
            i -= 1;
            while i >= 0 && text[i as usize] != open {
                out.insert(0, text[i as usize]);
                i -= 1;
            }
            i -= 1;
        } else if c == '.' && !out.is_empty() {
            out.insert(0, '.');
            i -= 1;
        } else {
            break;
        }
    }
    let s: String = out.into_iter().collect();
    if s.is_empty() || s.chars().all(|c| c.is_ascii_digit()) {
        None
    } else {
        Some(s)
    }
}

/// A string token whose closing quote is really a closer (not an escaped `''` at the end).
fn closed_string(text: &str) -> bool {
    let body = text.trim_start_matches(['N', 'n']);
    let inner: Vec<char> = body.chars().collect();
    if inner.len() < 2 || inner[inner.len() - 1] != '\'' {
        return false;
    }
    // Count trailing quotes: an odd number ends the literal.
    let trailing = inner.iter().rev().take_while(|c| **c == '\'').count();
    trailing % 2 == 1 && !(inner.len() == 2 && trailing == 1 && false)
}

pub fn analyze_context(sql: &str, caret: usize) -> SqlContext {
    let text: Vec<char> = sql.chars().collect();
    let caret = caret.min(text.len());
    let analysis = scope::analyze(sql);
    let (partial, word_start) = partial_word(&text, caret);
    let mut ctx = SqlContext { partial: partial.clone(), word_start, ..Default::default() };

    // Inside a string literal: what's typed so far is the text after the opening quote.
    if let Some(t) = analysis.tokens.iter().find(|t| t.kind == TokenKind::String && t.start < caret && (caret < t.end || !t.text.ends_with('\'') || t.text.len() == 1 || (caret == t.end && !closed_string(&t.text)))) {
        let open = t.start + if t.text.starts_with('N') || t.text.starts_with('n') { 2 } else { 1 };
        if caret >= open {
            ctx.in_string = Some(t.start);
            ctx.partial = text[open..caret].iter().collect();
            ctx.word_start = open;
        }
    }

    let statement = analysis.statement_at(caret);
    let block = statement.and_then(|s| analysis.block_at(s, caret));
    ctx.statement = statement;
    ctx.block = block;
    if let Some(b) = block {
        for src in analysis.visible_sources(b) {
            if let (Some(r), Some(t)) = (src.reference(), src.table_name()) {
                ctx.table_aliases.insert(r, &t);
            }
        }
        if let Some(st) = statement {
            for cte in &analysis.statements[st].ctes {
                ctx.table_aliases.insert(&cte.name, &cte.name);
            }
        }
    }
    ctx.available_aliases = ctx.table_aliases.keys();

    if sql.trim().is_empty() {
        ctx.kind = Some(SqlContextType::Unknown);
        ctx.analysis = analysis;
        return ctx;
    }

    // `alias.|` / `schema.table.|`
    if let Some(prefix) = qualifier_before(&text, word_start) {
        ctx.kind = Some(SqlContextType::ColumnAfterDot);
        ctx.table_prefix = prefix;
        ctx.expecting_column_name = true;
        ctx.clause = block.and_then(|b| analysis.clause_at(b, caret).map(|c| c.kind));
        ctx.analysis = analysis;
        return ctx;
    }

    let Some(b) = block else {
        ctx.kind = Some(SqlContextType::General);
        ctx.expecting_keyword = true;
        ctx.analysis = analysis;
        return ctx;
    };
    let clause = analysis.clause_at(b, caret).cloned();
    ctx.clause = clause.as_ref().map(|c| c.kind);
    let tb = analysis.token_before(caret);
    // Token touching the caret from the left (the word being typed), if any.
    let touching = tb.filter(|&i| analysis.tokens[i].end == caret && !partial.is_empty());
    // Last complete token before the typed word.
    let prev = if touching.is_some() { tb.and_then(|i| i.checked_sub(1)) } else { tb };
    let prev_tok = prev.map(|i| &analysis.tokens[i]);
    let blk = &analysis.blocks[b];

    let kind = match clause.as_ref().map(|c| c.kind) {
        Some(ClauseKind::SelectList) => {
            ctx.expecting_column_name = true;
            ctx.last_keyword = "SELECT".into();
            if let Some(i) = tb {
                let t = &analysis.tokens[i];
                if t.punct("*") && t.end == caret {
                    let mut start = t.start;
                    if i >= 2 && analysis.tokens[i - 1].punct(".") && analysis.tokens[i - 2].is_name() {
                        start = analysis.tokens[i - 2].start;
                        ctx.table_prefix = analysis.tokens[i - 2].value.clone();
                    }
                    ctx.after_star = Some(start);
                }
            }
            SqlContextType::SelectList
        }
        Some(ClauseKind::From) | Some(ClauseKind::JoinTable) | Some(ClauseKind::Using) => {
            ctx.last_keyword = if clause.as_ref().is_some_and(|c| c.kind == ClauseKind::From) { "FROM".into() } else { "JOIN".into() };
            from_context(&analysis, &mut ctx, b, caret, tb, touching, prev_tok)
        }
        Some(ClauseKind::On) => {
            ctx.expecting_column_name = true;
            ctx.last_keyword = "ON".into();
            // The source joined just before this ON.
            let kw = clause.as_ref().unwrap().keyword;
            ctx.joined_source = blk.sources.iter().enumerate().filter(|(_, s)| s.end <= kw).max_by_key(|(_, s)| s.start).map(|(i, _)| i);
            SqlContextType::JoinCondition
        }
        Some(ClauseKind::Where) | Some(ClauseKind::Having) | Some(ClauseKind::Limit) => {
            ctx.expecting_column_name = true;
            ctx.last_keyword = clause.as_ref().map(|c| analysis.tokens[c.keyword].upper()).unwrap_or_default();
            SqlContextType::WhereClause
        }
        Some(ClauseKind::GroupBy) | Some(ClauseKind::OrderBy) => {
            let kw_tok = &analysis.tokens[clause.as_ref().unwrap().keyword];
            ctx.last_keyword = format!("{} BY", kw_tok.upper());
            // "GROUP |" or "GROUP B|": only BY can follow.
            let by_present = clause.as_ref().unwrap().keyword + 1 < analysis.tokens.len() && analysis.tokens[clause.as_ref().unwrap().keyword + 1].is("BY");
            if !by_present && prev_tok.is_some_and(|t| t.is("GROUP") || t.is("ORDER")) {
                ctx.expecting_keyword = true;
                ctx.last_keyword = kw_tok.upper();
                SqlContextType::AfterGroupOrOrder
            } else {
                ctx.expecting_column_name = true;
                SqlContextType::WhereClause
            }
        }
        Some(ClauseKind::InsertTarget) => {
            ctx.expecting_table_name = true;
            if let Some(target) = &blk.target {
                let after_target = touching.is_none() && tb.is_some_and(|i| i + 1 == target.end) && caret > analysis.tokens[target.end - 1].end;
                let has_columns = blk.clauses.iter().any(|c| matches!(c.kind, ClauseKind::InsertColumns | ClauseKind::Values | ClauseKind::Set));
                ctx.after_insert_target = after_target && !has_columns && !analysis.blocks.iter().any(|x| x.parent == Some(b));
            }
            SqlContextType::InsertInto
        }
        Some(ClauseKind::InsertColumns) => {
            ctx.expecting_column_name = true;
            SqlContextType::InsertColumns
        }
        Some(ClauseKind::Values) => SqlContextType::InsertValues,
        Some(ClauseKind::Set) => {
            ctx.expecting_column_name = true;
            ctx.last_keyword = "SET".into();
            SqlContextType::UpdateSet
        }
        Some(ClauseKind::UpdateTarget) | Some(ClauseKind::DeleteTarget) => {
            ctx.expecting_table_name = true;
            // After the target name (with a space), the keyword SET/FROM/WHERE is expected.
            if let Some(target) = &blk.target {
                if touching.is_none() && caret > analysis.tokens[target.end - 1].end {
                    ctx.expecting_table_name = false;
                    ctx.expecting_keyword = true;
                    return finish(ctx, analysis, SqlContextType::AfterTableName);
                }
            }
            SqlContextType::UpdateTable
        }
        Some(ClauseKind::Into) => {
            ctx.expecting_table_name = true;
            SqlContextType::General
        }
        Some(ClauseKind::With) | Some(ClauseKind::Other) | None => {
            ctx.expecting_keyword = true;
            if blk.kind == BlockKind::Other && prev_tok.is_some_and(|t| t.is_any(&["FROM", "JOIN"])) {
                ctx.expecting_table_name = true;
                return finish(ctx, analysis, SqlContextType::FromClause);
            }
            SqlContextType::General
        }
    };
    finish(ctx, analysis, kind)
}

fn finish(mut ctx: SqlContext, analysis: Analysis, kind: SqlContextType) -> SqlContext {
    ctx.kind = Some(kind);
    ctx.analysis = analysis;
    ctx
}

/// Inside FROM / JOIN: typing a table, or past a table (and its alias) so a keyword is expected.
fn from_context(
    analysis: &Analysis,
    ctx: &mut SqlContext,
    block: usize,
    caret: usize,
    tb: Option<usize>,
    touching: Option<usize>,
    prev_tok: Option<&super::lexer::Token>,
) -> SqlContextType {
    let blk = &analysis.blocks[block];
    ctx.expecting_table_name = true;
    let Some(tbi) = tb else { return SqlContextType::FromClause };
    // Right after FROM / JOIN / a comma: a table name is expected.
    if prev_tok.is_none_or(|t| t.punct(",") || t.is_any(&["FROM", "JOIN", "APPLY", "USING", "INTO"])) {
        return SqlContextType::FromClause;
    }
    // The source whose tokens the caret follows.
    let Some(src) = blk.sources.iter().filter(|s| s.start <= tbi).max_by_key(|s| s.start) else {
        return SqlContextType::FromClause;
    };
    let last_tok = src.end.saturating_sub(1);
    let alias_tok = src.alias.as_ref().map(|_| last_tok);
    if let Some(ti) = touching {
        if alias_tok == Some(ti) {
            // "FROM Customers c WH|": the "alias" is really a keyword being typed.
            ctx.expecting_table_name = false;
            ctx.expecting_keyword = true;
            return SqlContextType::AfterTableAlias;
        }
        if ti < src.end {
            return SqlContextType::FromClause; // still typing the table name
        }
    }
    if matches!(src.kind, SourceKind::Derived(_)) && caret < analysis.tokens[last_tok].end {
        return SqlContextType::FromClause;
    }
    ctx.expecting_table_name = false;
    ctx.expecting_keyword = true;
    if src.alias.is_some() {
        SqlContextType::AfterTableAlias
    } else {
        SqlContextType::AfterTableName
    }
}

/// Every table named in FROM / JOIN / target positions, schema-qualified where written.
pub fn extract_table_names(sql: &str) -> Vec<String> {
    let a = scope::analyze(sql);
    let mut out: Vec<String> = Vec::new();
    for b in &a.blocks {
        for s in &b.sources {
            if let SourceKind::Table(parts) = &s.kind {
                let full = parts.join(".");
                if !out.iter().any(|x| x.eq_ignore_ascii_case(&full)) {
                    out.push(full);
                }
            }
        }
    }
    out
}

pub fn extract_table_aliases(sql: &str) -> Aliases {
    let a = scope::analyze(sql);
    let mut aliases = Aliases::default();
    for b in &a.blocks {
        for s in &b.sources {
            if let (Some(r), Some(t)) = (s.reference(), s.table_name()) {
                aliases.insert(r, &t);
            }
        }
    }
    aliases
}

/// Keep the lexer's notion of a bare word for callers that need it.
pub fn is_identifier_kind(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Word | TokenKind::QuotedIdent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_end(sql: &str) -> SqlContext {
        analyze_context(sql, sql.chars().count())
    }

    #[test]
    fn select_list_with_columns() {
        assert_eq!(analyze_context("SELECT Id, FROM Users", 11).context_type(), SqlContextType::SelectList);
    }

    #[test]
    fn from_clause() {
        assert_eq!(analyze_context("SELECT * FROM ", 14).context_type(), SqlContextType::FromClause);
    }

    #[test]
    fn where_clause() {
        assert_eq!(analyze_context("SELECT * FROM Users WHERE ", 25).context_type(), SqlContextType::WhereClause);
    }

    #[test]
    fn column_after_dot() {
        let ctx = analyze_context("SELECT u. FROM Users u", 9);
        assert_eq!(ctx.context_type(), SqlContextType::ColumnAfterDot);
        assert_eq!(ctx.table_prefix, "u");
        let ctx = at_end("SELECT * FROM [dbo].[Orders].");
        assert_eq!(ctx.table_prefix, "dbo.Orders");
    }

    #[test]
    fn simple_and_as_aliases() {
        assert_eq!(analyze_context("SELECT u.Name FROM Users u WHERE ", 33).table_aliases.get("u"), Some("Users"));
        assert_eq!(analyze_context("SELECT p.Id FROM Products AS p WHERE ", 37).table_aliases.get("p"), Some("Products"));
    }

    #[test]
    fn join_aliases_and_condition() {
        let sql = "SELECT o.Id FROM Orders o INNER JOIN Customers c ON ";
        let ctx = at_end(sql);
        assert_eq!(ctx.table_aliases.get("o"), Some("Orders"));
        assert_eq!(ctx.table_aliases.get("c"), Some("Customers"));
        assert_eq!(ctx.context_type(), SqlContextType::JoinCondition);
        assert_eq!(ctx.joined_source, Some(1));
    }

    #[test]
    fn empty_and_zero_caret_do_not_panic() {
        analyze_context("", 0);
        analyze_context("SELECT * FROM Users", 0);
        analyze_context("SELECT * FROM Users", 999);
    }

    #[test]
    fn insert_contexts() {
        assert_eq!(analyze_context("INSERT INTO ", 12).context_type(), SqlContextType::InsertInto);
        let ctx = at_end("INSERT INTO dbo.T ");
        assert_eq!(ctx.context_type(), SqlContextType::InsertInto);
        assert!(ctx.after_insert_target);
        assert_eq!(at_end("INSERT INTO T (a, ").context_type(), SqlContextType::InsertColumns);
        assert_eq!(at_end("INSERT INTO T (a) VALUES (").context_type(), SqlContextType::InsertValues);
    }

    #[test]
    fn update_contexts() {
        assert_eq!(at_end("UPDATE ").context_type(), SqlContextType::UpdateTable);
        assert_eq!(at_end("UPDATE Users ").context_type(), SqlContextType::AfterTableName);
        assert_eq!(at_end("UPDATE Users SET ").context_type(), SqlContextType::UpdateSet);
        let ctx = at_end("UPDATE u SET Name = 'x' FROM Users u WHERE u.");
        assert_eq!(ctx.context_type(), SqlContextType::ColumnAfterDot);
        assert_eq!(ctx.table_aliases.get("u"), Some("Users"));
    }

    #[test]
    fn extract_table_names_finds_tables() {
        let tables = extract_table_names("SELECT * FROM Users u JOIN Orders o ON u.Id = o.UserId");
        assert_eq!(tables, ["Users", "Orders"]);
    }

    #[test]
    fn aliases_after_caret_are_available() {
        assert!(analyze_context("SELECT  FROM Users u", 7).table_aliases.contains("u"));
    }

    #[test]
    fn comma_separated_tables_all_get_aliases() {
        let a = extract_table_aliases("SELECT * FROM a, b c, dbo.d AS e");
        assert_eq!(a.get("a"), Some("a"));
        assert_eq!(a.get("c"), Some("b"));
        assert_eq!(a.get("e"), Some("dbo.d"));
    }

    #[test]
    fn after_table_name_and_alias() {
        let ctx = at_end("SELECT * FROM Customers c ");
        assert_eq!(ctx.context_type(), SqlContextType::AfterTableAlias);
        assert!(ctx.expecting_keyword);
        assert_eq!(at_end("SELECT * FROM Customers ").context_type(), SqlContextType::AfterTableName);
        assert_eq!(at_end("SELECT * FROM Customers c WH").context_type(), SqlContextType::AfterTableAlias);
        assert_eq!(at_end("SELECT * FROM Cust").context_type(), SqlContextType::FromClause);
        assert_eq!(at_end("SELECT * FROM a, ").context_type(), SqlContextType::FromClause);
        assert_eq!(at_end("SELECT * FROM a LEFT JOIN ").context_type(), SqlContextType::FromClause);
    }

    #[test]
    fn group_expects_by() {
        assert_eq!(at_end("SELECT * FROM t GROUP ").context_type(), SqlContextType::AfterGroupOrOrder);
        assert_eq!(at_end("SELECT * FROM t ORDER B").context_type(), SqlContextType::AfterGroupOrOrder);
        assert_eq!(at_end("SELECT * FROM t ORDER BY ").context_type(), SqlContextType::WhereClause);
    }

    #[test]
    fn subquery_scope_sees_outer_sources() {
        let sql = "SELECT * FROM Orders o WHERE EXISTS (SELECT 1 FROM Items i WHERE i.OrderId = ";
        let ctx = at_end(sql);
        assert_eq!(ctx.context_type(), SqlContextType::WhereClause);
        assert_eq!(ctx.table_aliases.get("o"), Some("Orders"));
        assert_eq!(ctx.table_aliases.get("i"), Some("Items"));
    }

    #[test]
    fn cte_and_derived_aliases() {
        let ctx = at_end("WITH c AS (SELECT Id FROM T) SELECT * FROM c JOIN (SELECT 1 AS x) d ON d.");
        assert_eq!(ctx.context_type(), SqlContextType::ColumnAfterDot);
        assert_eq!(ctx.table_aliases.get("c"), Some("c"));
        assert!(ctx.available_aliases.iter().any(|a| a == "c"));
    }

    #[test]
    fn star_expansion_position() {
        let ctx = at_end("SELECT t.* FROM T t");
        let ctx2 = analyze_context("SELECT t.* FROM T t", 10);
        assert!(ctx.after_star.is_none());
        assert_eq!(ctx2.after_star, Some(7));
        assert_eq!(ctx2.table_prefix, "t");
    }

    #[test]
    fn inside_a_string_literal() {
        let ctx = analyze_context("SELECT * FROM T WHERE Id = '085d", 32);
        assert_eq!(ctx.in_string, Some(27));
        assert_eq!(ctx.partial, "085d");
        let ctx = analyze_context("SELECT * FROM T WHERE Id = 'abc' AND ", 37);
        assert!(ctx.in_string.is_none());
        let ctx = analyze_context("SELECT * FROM T WHERE Id = N'ab", 31);
        assert_eq!(ctx.partial, "ab");
    }

    #[test]
    fn second_statement_uses_its_own_scope() {
        let sql = "SELECT * FROM A a;\nSELECT * FROM B b WHERE ";
        let ctx = at_end(sql);
        assert!(ctx.table_aliases.contains("b"));
        assert!(!ctx.table_aliases.contains("a"));
    }
}
