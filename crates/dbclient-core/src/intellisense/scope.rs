//! Statement structure for completion: a tolerant recursive-descent pass over the tokens that
//! finds statements, their CTEs, every query block (including subqueries and derived tables),
//! each block's sources (tables, CTEs, derived tables, temp tables, functions) with aliases, the
//! clause the caret is in, batch-level `DECLARE`d variables and temp tables.
//!
//! It never fails: unknown constructs are skipped, missing closers are taken as the end of the
//! text, so a half-typed statement still analyzes as far as it goes.

use super::lexer::{tokenize, Token, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseKind {
    /// `WITH name AS (...)` list, outside the CTE bodies.
    With,
    SelectList,
    Into,
    From,
    /// Right after a JOIN / APPLY keyword, before ON: a table is expected.
    JoinTable,
    On,
    Where,
    GroupBy,
    Having,
    OrderBy,
    /// LIMIT / OFFSET / FETCH.
    Limit,
    InsertTarget,
    InsertColumns,
    Values,
    UpdateTarget,
    Set,
    DeleteTarget,
    /// MERGE ... USING <source>
    Using,
    /// Anything else (DECLARE, EXEC, DDL, ...).
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    pub kind: ClauseKind,
    /// Index of the keyword token that opened the clause.
    pub keyword: usize,
    /// First token index of the clause body (after the keyword) and one past the last.
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    Table(Vec<String>),
    Cte(String),
    Derived(usize),
    Temp(String),
    /// A table-valued function call; the name parts.
    Function(Vec<String>),
    /// `OPENJSON(...)`, `VALUES (...)` or anything else with an alias.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub kind: SourceKind,
    pub alias: Option<String>,
    /// Explicit column list after the alias: `... AS d (a, b)`.
    pub columns: Vec<String>,
    /// Token index range of the reference.
    pub start: usize,
    pub end: usize,
    /// Joined in with JOIN / APPLY (vs. the first source or a comma).
    pub joined: bool,
}

impl Source {
    /// The name the source can be referred to by without an alias (last name part), if any.
    pub fn bare_name(&self) -> Option<&str> {
        match &self.kind {
            SourceKind::Table(p) | SourceKind::Function(p) => p.last().map(|s| s.as_str()),
            SourceKind::Cte(n) | SourceKind::Temp(n) => Some(n),
            _ => None,
        }
    }
    /// How this source is referenced in expressions: alias, else bare name.
    pub fn reference(&self) -> Option<&str> {
        self.alias.as_deref().or_else(|| self.bare_name())
    }
    pub fn table_name(&self) -> Option<String> {
        match &self.kind {
            SourceKind::Table(p) => Some(p.join(".")),
            SourceKind::Temp(n) | SourceKind::Cte(n) => Some(n.clone()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Projection {
    /// Output name (alias or the last part of a column path); `None` for unnamed expressions.
    pub name: Option<String>,
    pub star: bool,
    /// `t.*` → `Some("t")`.
    pub star_prefix: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Select,
    Insert,
    Update,
    Delete,
    Merge,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub parent: Option<usize>,
    pub depth: usize,
    /// Token index range (end exclusive). For a subquery, inside the parens.
    pub start: usize,
    pub end: usize,
    pub sources: Vec<Source>,
    pub clauses: Vec<Clause>,
    pub projection: Vec<Projection>,
    /// INSERT / UPDATE / DELETE / MERGE target.
    pub target: Option<Source>,
    /// INSERT (col, ...) list.
    pub insert_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cte {
    pub name: String,
    pub columns: Vec<String>,
    pub block: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub start: usize,
    pub end: usize,
    pub ctes: Vec<Cte>,
    /// Indices into `Analysis::blocks`; the first is the statement's own.
    pub blocks: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempTable {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub tokens: Vec<Token>,
    pub statements: Vec<Statement>,
    pub blocks: Vec<Block>,
    pub variables: Vec<Variable>,
    pub temp_tables: Vec<TempTable>,
}

const STATEMENT_STARTS: &[&str] = &[
    "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "ALTER", "DROP", "DECLARE", "SET", "EXEC", "EXECUTE", "MERGE",
    "TRUNCATE", "BEGIN", "COMMIT", "ROLLBACK", "IF", "WHILE", "PRINT", "USE", "GRANT", "REVOKE", "CALL", "SHOW", "DESCRIBE",
    "EXPLAIN", "PRAGMA", "VACUUM", "ATTACH", "DETACH", "RETURN", "THROW", "RAISERROR", "GO",
];

const CLAUSE_STOPS: &[&str] = &[
    "FROM", "WHERE", "GROUP", "HAVING", "ORDER", "UNION", "EXCEPT", "INTERSECT", "LIMIT", "OFFSET", "FETCH", "INTO", "WINDOW",
    "RETURNING", "FOR",
];

const JOIN_WORDS: &[&str] = &["JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "OUTER", "NATURAL", "STRAIGHT_JOIN"];

pub fn is_keyword(word: &str) -> bool {
    let u = word.to_uppercase();
    STATEMENT_STARTS.contains(&u.as_str())
        || CLAUSE_STOPS.contains(&u.as_str())
        || JOIN_WORDS.contains(&u.as_str())
        || matches!(
            u.as_str(),
            "ON" | "AND" | "OR" | "NOT" | "IN" | "IS" | "NULL" | "AS" | "BY" | "ASC" | "DESC" | "DISTINCT" | "TOP" | "ALL" | "ANY"
                | "SOME" | "EXISTS" | "BETWEEN" | "LIKE" | "CASE" | "WHEN" | "THEN" | "ELSE" | "END" | "VALUES" | "USING" | "APPLY"
                | "TABLE" | "VIEW" | "INDEX" | "PROCEDURE" | "FUNCTION" | "TRIGGER" | "MATCHED" | "OUTPUT" | "OVER" | "PARTITION"
                | "ROWS" | "ONLY" | "NEXT" | "FIRST" | "LAST" | "NULLS" | "WITHIN" | "PIVOT" | "UNPIVOT" | "TABLESAMPLE" | "NOLOCK"
                | "READUNCOMMITTED" | "DEFAULT" | "ESCAPE" | "COLLATE" | "CAST" | "CONVERT" | "TRY_CAST" | "IIF" | "INTO"
        )
}

struct Parser<'a> {
    t: &'a [Token],
    pos: usize,
    /// Hard stop (exclusive) for the current statement.
    limit: usize,
    blocks: Vec<Block>,
    statement_blocks: Vec<usize>,
    ctes: Vec<Cte>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Token> {
        if self.pos < self.limit {
            Some(&self.t[self.pos])
        } else {
            None
        }
    }
    fn peek_at(&self, n: usize) -> Option<&'a Token> {
        let i = self.pos + n;
        if i < self.limit {
            Some(&self.t[i])
        } else {
            None
        }
    }
    fn at_end(&self) -> bool {
        self.pos >= self.limit
    }
    fn is_word(&self, w: &str) -> bool {
        self.peek().is_some_and(|t| t.is(w))
    }
    fn is_punct(&self, p: &str) -> bool {
        self.peek().is_some_and(|t| t.punct(p))
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn new_block(&mut self, kind: BlockKind, parent: Option<usize>, start: usize) -> usize {
        let depth = parent.map(|p| self.blocks[p].depth + 1).unwrap_or(0);
        self.blocks.push(Block {
            kind,
            parent,
            depth,
            start,
            end: start,
            sources: Vec::new(),
            clauses: Vec::new(),
            projection: Vec::new(),
            target: None,
            insert_columns: Vec::new(),
        });
        let id = self.blocks.len() - 1;
        self.statement_blocks.push(id);
        id
    }

    fn open_clause(&mut self, block: usize, kind: ClauseKind, keyword: usize) {
        if let Some(last) = self.blocks[block].clauses.last_mut() {
            if last.end < keyword {
                last.end = keyword;
            }
        }
        self.blocks[block].clauses.push(Clause { kind, keyword, start: keyword + 1, end: keyword + 1 });
    }

    fn close_clause(&mut self, block: usize) {
        let end = self.pos;
        if let Some(last) = self.blocks[block].clauses.last_mut() {
            if last.end < end {
                last.end = end;
            }
        }
    }

    /// Skip a balanced parenthesized group; a SELECT inside it becomes a child block.
    fn skip_parens(&mut self, parent: usize) {
        if !self.is_punct("(") {
            return;
        }
        self.pos += 1;
        if self.is_word("SELECT") || self.is_word("WITH") {
            self.parse_query(Some(parent));
        }
        self.skip_until_close(parent);
    }

    /// Skip tokens until the matching `)` (consumed) or end; nested parens handled, nested SELECTs
    /// become child blocks.
    fn skip_until_close(&mut self, parent: usize) {
        while let Some(t) = self.peek() {
            if t.punct(")") {
                self.pos += 1;
                return;
            }
            if t.punct("(") {
                self.skip_parens(parent);
                continue;
            }
            if t.is("SELECT") {
                self.parse_query(Some(parent));
                continue;
            }
            self.pos += 1;
        }
    }

    /// Statement-level: WITH ... then the main statement.
    fn parse_statement(&mut self) {
        if self.is_word("WITH") {
            self.parse_ctes();
        }
        let Some(t) = self.peek() else { return };
        let u = t.upper();
        match u.as_str() {
            "SELECT" => {
                self.parse_query(None);
            }
            "INSERT" | "REPLACE" => self.parse_insert(),
            "UPDATE" => self.parse_update(),
            "DELETE" => self.parse_delete(),
            "MERGE" => self.parse_merge(),
            _ => self.parse_other(),
        }
        // Anything left over (e.g. after a subquery closes at statement level) is scanned generically.
        if !self.at_end() {
            self.parse_other();
        }
    }

    fn parse_ctes(&mut self) {
        let with_block = self.new_block(BlockKind::Other, None, self.pos);
        let kw = self.pos;
        self.pos += 1; // WITH
        self.eat_word("RECURSIVE");
        self.open_clause(with_block, ClauseKind::With, kw);
        loop {
            let Some(name) = self.peek() else { break };
            if !name.is_name() || name.is("SELECT") {
                break;
            }
            let cte_name = name.value.clone();
            self.pos += 1;
            let mut columns = Vec::new();
            if self.is_punct("(") {
                columns = self.parse_name_list();
            }
            let mut block = None;
            if self.eat_word("AS") && self.is_punct("(") {
                self.pos += 1;
                if self.is_word("SELECT") || self.is_word("WITH") {
                    block = Some(self.parse_query(Some(with_block)));
                }
                self.skip_until_close(with_block);
            }
            self.ctes.push(Cte { name: cte_name, columns, block });
            if self.is_punct(",") {
                self.pos += 1;
                continue;
            }
            break;
        }
        self.close_clause(with_block);
        self.blocks[with_block].end = self.pos;
    }

    /// `( a, b, c )` → names; consumes the parens.
    fn parse_name_list(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.is_punct("(") {
            return out;
        }
        self.pos += 1;
        while let Some(t) = self.peek() {
            if t.punct(")") {
                self.pos += 1;
                break;
            }
            if t.is_name() {
                out.push(t.value.clone());
            }
            self.pos += 1;
        }
        out
    }

    /// A SELECT (with UNION chain). Returns the block id. Stops at `)`, a statement start or end.
    fn parse_query(&mut self, parent: Option<usize>) -> usize {
        let block = self.new_block(BlockKind::Select, parent, self.pos);
        if self.is_word("WITH") {
            // Nested WITH inside a subquery: treat its CTEs as statement CTEs.
            let saved = self.statement_blocks.len();
            self.parse_ctes();
            let _ = saved;
        }
        self.parse_select_body(block);
        // UNION / EXCEPT / INTERSECT chain: siblings share the parent; the first block's projection wins.
        while self.peek().is_some_and(|t| t.is_any(&["UNION", "EXCEPT", "INTERSECT"])) {
            self.pos += 1;
            self.eat_word("ALL");
            self.eat_word("DISTINCT");
            if self.is_word("SELECT") {
                let sibling = self.new_block(BlockKind::Select, parent, self.pos);
                self.parse_select_body(sibling);
            } else if self.is_punct("(") {
                self.skip_parens(parent.unwrap_or(block));
            } else {
                break;
            }
        }
        self.blocks[block].end = self.pos;
        block
    }

    fn parse_select_body(&mut self, block: usize) {
        let kw = self.pos;
        if !self.eat_word("SELECT") {
            return;
        }
        self.open_clause(block, ClauseKind::SelectList, kw);
        // Modifiers.
        loop {
            if self.eat_word("DISTINCT") || self.eat_word("ALL") {
                continue;
            }
            if self.is_word("TOP") {
                self.pos += 1;
                if self.is_punct("(") {
                    self.skip_parens(block);
                } else {
                    self.pos += 1;
                }
                self.eat_word("PERCENT");
                if self.is_word("WITH") {
                    self.pos += 1;
                    self.eat_word("TIES");
                }
                continue;
            }
            break;
        }
        self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
        self.parse_projection(block);
        self.close_clause(block);

        while let Some(t) = self.peek() {
            if t.punct(")") {
                return;
            }
            let u = t.upper();
            match u.as_str() {
                "INTO" => {
                    let kw = self.pos;
                    self.pos += 1;
                    self.open_clause(block, ClauseKind::Into, kw);
                    if let Some(n) = self.peek() {
                        if n.kind == TokenKind::TempName {
                            self.pos += 1;
                        } else if n.is_name() {
                            self.parse_qualified_name();
                        }
                    }
                    self.close_clause(block);
                }
                "FROM" => {
                    let kw = self.pos;
                    self.pos += 1;
                    self.open_clause(block, ClauseKind::From, kw);
                    self.parse_sources(block);
                    self.close_clause(block);
                }
                "WHERE" => self.parse_expr_clause(block, ClauseKind::Where),
                "GROUP" => self.parse_expr_clause(block, ClauseKind::GroupBy),
                "HAVING" => self.parse_expr_clause(block, ClauseKind::Having),
                "ORDER" => self.parse_expr_clause(block, ClauseKind::OrderBy),
                "LIMIT" | "OFFSET" | "FETCH" => self.parse_expr_clause(block, ClauseKind::Limit),
                "WINDOW" | "FOR" | "OPTION" => self.parse_expr_clause(block, ClauseKind::Other),
                "UNION" | "EXCEPT" | "INTERSECT" => return,
                _ => {
                    if is_statement_boundary(t) {
                        return;
                    }
                    if self.is_trailing_word() {
                        self.pos += 1;
                        continue;
                    }
                    // Unexpected token: treat as part of an expression tail.
                    self.parse_expr_clause(block, ClauseKind::Other);
                }
            }
        }
    }

    /// A lone non-keyword word with nothing after it: a keyword being typed, not a new clause.
    fn is_trailing_word(&self) -> bool {
        self.pos + 1 >= self.limit && self.peek().is_some_and(|t| t.kind == TokenKind::Word && !is_keyword(&t.value))
    }

    /// A clause whose body is an expression list (WHERE, GROUP BY, ORDER BY, HAVING, ON, SET ...).
    fn parse_expr_clause(&mut self, block: usize, kind: ClauseKind) {
        let kw = self.pos;
        self.pos += 1;
        if matches!(kind, ClauseKind::GroupBy | ClauseKind::OrderBy) {
            self.eat_word("BY");
        }
        self.open_clause(block, kind, kw);
        self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
        self.skip_expression(block, &[]);
        self.close_clause(block);
    }

    /// Skip an expression until a clause keyword at depth 0, `)`, `,` when `stop_at_comma`, or a
    /// statement boundary. Subqueries become child blocks.
    fn skip_expression(&mut self, block: usize, extra_stops: &[&str]) {
        while let Some(t) = self.peek() {
            if t.punct(")") {
                return;
            }
            if t.punct("(") {
                self.skip_parens(block);
                continue;
            }
            if t.kind == TokenKind::Word {
                let u = t.upper();
                if CLAUSE_STOPS.contains(&u.as_str()) && u != "FOR" || is_statement_boundary(t) || extra_stops.contains(&u.as_str()) {
                    // FOR is a stop only as FOR XML/JSON/UPDATE at clause level; inside expressions it's rare.
                    return;
                }
                if JOIN_WORDS.contains(&u.as_str()) || u == "ON" || u == "APPLY" {
                    return;
                }
                if u == "WHEN" || u == "SET" || u == "VALUES" || u == "USING" {
                    return;
                }
                // `SELECT` here means a subquery without parens (invalid) or a new statement.
                if u == "SELECT" {
                    return;
                }
            }
            self.pos += 1;
        }
    }

    fn parse_projection(&mut self, block: usize) {
        loop {
            let item_start = self.pos;
            let mut last_name: Option<String> = None;
            let mut prev_name: Option<String> = None;
            let mut star = false;
            let mut star_prefix: Option<String> = None;
            let mut alias: Option<String> = None;
            let mut depth = 0;
            let mut tokens_seen = 0;
            while let Some(t) = self.peek() {
                if depth == 0 {
                    if t.punct(",") || t.punct(")") {
                        break;
                    }
                    if t.kind == TokenKind::Word {
                        let u = t.upper();
                        if (CLAUSE_STOPS.contains(&u.as_str()) || is_statement_boundary(t)) && u != "FOR" {
                            break;
                        }
                        if u == "AS" {
                            self.pos += 1;
                            if let Some(a) = self.peek() {
                                if a.is_name() {
                                    alias = Some(a.value.clone());
                                    self.pos += 1;
                                }
                            }
                            continue;
                        }
                    }
                }
                if t.punct("(") {
                    depth += 1;
                    // Subquery in the select list.
                    if self.peek_at(1).is_some_and(|n| n.is("SELECT")) {
                        self.pos += 1;
                        self.parse_query(Some(block));
                        self.skip_until_close(block);
                        depth -= 1;
                        tokens_seen += 1;
                        last_name = None;
                        continue;
                    }
                } else if t.punct(")") {
                    depth -= 1;
                }
                if depth == 0 {
                    if t.punct("*") {
                        star = true;
                        star_prefix = if self.pos > item_start && self.t[self.pos - 1].punct(".") { last_name.clone() } else { None };
                        last_name = None;
                    } else if t.is_name() && t.kind != TokenKind::Variable {
                        // `expr alias` (no AS): a name following a complete name/expression.
                        let after_dot = self.pos > item_start && self.t[self.pos - 1].punct(".");
                        if !after_dot && tokens_seen > 0 && !self.t[self.pos - 1].kind.eq(&TokenKind::Punct) && !is_keyword(&t.value) {
                            alias = Some(t.value.clone());
                        }
                        prev_name = last_name.take();
                        last_name = Some(t.value.clone());
                    } else if !t.punct(".") {
                        last_name = None;
                    }
                }
                tokens_seen += 1;
                self.pos += 1;
            }
            let _ = prev_name;
            if tokens_seen > 0 || star {
                self.blocks[block].projection.push(Projection {
                    name: alias.or(if star { None } else { last_name }),
                    star,
                    star_prefix,
                });
            }
            if self.is_punct(",") {
                self.pos += 1;
                continue;
            }
            break;
        }
    }

    /// `a.b.c` parts; consumes them. Stops at a trailing dot (returns parts so far).
    fn parse_qualified_name(&mut self) -> Vec<String> {
        let mut parts = Vec::new();
        while let Some(t) = self.peek() {
            if t.is_name() && !is_keyword(&t.value) || (parts.is_empty() && t.is_name()) {
                parts.push(t.value.clone());
                self.pos += 1;
                if self.is_punct(".") {
                    self.pos += 1;
                    continue;
                }
            }
            break;
        }
        parts
    }

    /// Sources of a FROM clause (or the FROM of UPDATE/DELETE): comma list and JOIN chain.
    fn parse_sources(&mut self, block: usize) {
        loop {
            if !self.parse_table_ref(block, false) {
                // Dangling comma / JOIN with nothing after: a table is expected here.
                return;
            }
            // Table hints: WITH (NOLOCK)
            if self.is_word("WITH") && self.peek_at(1).is_some_and(|t| t.punct("(")) {
                self.pos += 1;
                self.skip_parens(block);
            }
            loop {
                let Some(t) = self.peek() else { return };
                if t.punct(",") {
                    self.pos += 1;
                    break;
                }
                let u = t.upper();
                if JOIN_WORDS.contains(&u.as_str()) || u == "APPLY" {
                    let kw = self.pos;
                    while self.peek().is_some_and(|t| t.is_any(JOIN_WORDS) || t.is("APPLY")) {
                        self.pos += 1;
                    }
                    self.open_clause(block, ClauseKind::JoinTable, kw);
                    self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
                    let ok = self.parse_table_ref(block, true);
                    self.close_clause(block);
                    if !ok {
                        return;
                    }
                    if self.is_word("WITH") && self.peek_at(1).is_some_and(|t| t.punct("(")) {
                        self.pos += 1;
                        self.skip_parens(block);
                    }
                    if self.is_word("ON") {
                        let kw = self.pos;
                        self.pos += 1;
                        self.open_clause(block, ClauseKind::On, kw);
                        self.skip_expression(block, &[]);
                        self.close_clause(block);
                    } else if self.is_word("USING") {
                        self.pos += 1;
                        if self.is_punct("(") {
                            self.skip_parens(block);
                        }
                    }
                    continue;
                }
                if u == "PIVOT" || u == "UNPIVOT" {
                    self.pos += 1;
                    if self.is_punct("(") {
                        self.skip_parens(block);
                    }
                    self.parse_alias_opt();
                    continue;
                }
                return;
            }
        }
    }

    /// `[AS] alias [(cols)]`; returns the alias.
    fn parse_alias_opt(&mut self) -> (Option<String>, Vec<String>) {
        let mut alias = None;
        if self.eat_word("AS") {
            if let Some(t) = self.peek() {
                if t.is_name() {
                    alias = Some(t.value.clone());
                    self.pos += 1;
                }
            }
        } else if let Some(t) = self.peek() {
            if t.is_name() && !is_keyword(&t.value) && t.kind != TokenKind::Variable {
                alias = Some(t.value.clone());
                self.pos += 1;
            }
        }
        let mut columns = Vec::new();
        if alias.is_some() && self.is_punct("(") {
            columns = self.parse_name_list();
        }
        (alias, columns)
    }

    /// One table reference. Returns false when there is nothing usable at the cursor.
    fn parse_table_ref(&mut self, block: usize, joined: bool) -> bool {
        let start = self.pos;
        let Some(t) = self.peek() else { return false };
        if t.punct("(") {
            self.pos += 1;
            let kind = if self.is_word("SELECT") || self.is_word("WITH") {
                let child = self.parse_query(Some(block));
                SourceKind::Derived(child)
            } else if self.is_word("VALUES") {
                SourceKind::Other
            } else {
                // Parenthesized join group: parse its sources into this block.
                self.parse_sources(block);
                self.skip_until_close(block);
                return true;
            };
            self.skip_until_close(block);
            let (alias, columns) = self.parse_alias_opt();
            self.blocks[block].sources.push(Source { kind, alias, columns, start, end: self.pos, joined });
            return true;
        }
        if t.kind == TokenKind::TempName {
            let name = t.value.clone();
            self.pos += 1;
            let (alias, columns) = self.parse_alias_opt();
            self.blocks[block].sources.push(Source { kind: SourceKind::Temp(name), alias, columns, start, end: self.pos, joined });
            return true;
        }
        if t.kind == TokenKind::Variable {
            // Table variable.
            let name = t.value.clone();
            self.pos += 1;
            let (alias, columns) = self.parse_alias_opt();
            self.blocks[block].sources.push(Source { kind: SourceKind::Temp(name), alias, columns, start, end: self.pos, joined });
            return true;
        }
        if !t.is_name() || is_keyword(&t.value) {
            return false;
        }
        let parts = self.parse_qualified_name();
        if parts.is_empty() {
            return false;
        }
        if self.is_punct("(") {
            // Table-valued function.
            self.skip_parens(block);
            let (alias, columns) = self.parse_alias_opt();
            self.blocks[block].sources.push(Source { kind: SourceKind::Function(parts), alias, columns, start, end: self.pos, joined });
            return true;
        }
        let kind = if parts.len() == 1 && self.ctes.iter().any(|c| c.name.eq_ignore_ascii_case(&parts[0])) {
            SourceKind::Cte(parts[0].clone())
        } else {
            SourceKind::Table(parts)
        };
        let (alias, columns) = self.parse_alias_opt();
        self.blocks[block].sources.push(Source { kind, alias, columns, start, end: self.pos, joined });
        true
    }

    fn parse_target(&mut self, block: usize) -> Option<Source> {
        let start = self.pos;
        let t = self.peek()?;
        let kind = if t.kind == TokenKind::TempName || t.kind == TokenKind::Variable {
            self.pos += 1;
            SourceKind::Temp(t.value.clone())
        } else if t.is_name() && !is_keyword(&t.value) {
            let parts = self.parse_qualified_name();
            if parts.is_empty() {
                return None;
            }
            if parts.len() == 1 && self.ctes.iter().any(|c| c.name.eq_ignore_ascii_case(&parts[0])) {
                SourceKind::Cte(parts[0].clone())
            } else {
                SourceKind::Table(parts)
            }
        } else {
            return None;
        };
        let _ = block;
        let (alias, _) = self.parse_alias_opt();
        Some(Source { kind, alias, columns: Vec::new(), start, end: self.pos, joined: false })
    }

    fn parse_insert(&mut self) {
        let block = self.new_block(BlockKind::Insert, None, self.pos);
        let kw = self.pos;
        self.pos += 1; // INSERT / REPLACE
        self.eat_word("IGNORE");
        self.eat_word("INTO");
        self.open_clause(block, ClauseKind::InsertTarget, kw);
        self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
        if let Some(target) = self.parse_target(block) {
            self.blocks[block].sources.push(target.clone());
            self.blocks[block].target = Some(target);
        }
        self.close_clause(block);
        if self.is_punct("(") {
            let kw = self.pos;
            self.open_clause(block, ClauseKind::InsertColumns, kw);
            let cols = self.parse_name_list();
            self.blocks[block].insert_columns = cols;
            self.close_clause(block);
        }
        if self.is_word("OUTPUT") {
            self.parse_expr_clause(block, ClauseKind::Other);
        }
        if self.is_word("VALUES") {
            let kw = self.pos;
            self.pos += 1;
            self.open_clause(block, ClauseKind::Values, kw);
            while let Some(t) = self.peek() {
                if t.punct("(") {
                    self.skip_parens(block);
                } else if t.punct(",") {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.close_clause(block);
        } else if self.is_word("SELECT") || self.is_word("WITH") {
            self.parse_query(Some(block));
        } else if self.is_word("DEFAULT") {
            self.pos += 1;
            self.eat_word("VALUES");
        } else if self.is_word("EXEC") || self.is_word("EXECUTE") {
            self.parse_expr_clause(block, ClauseKind::Other);
        } else if self.is_word("SET") {
            // MySQL INSERT ... SET a = 1
            self.parse_expr_clause(block, ClauseKind::Set);
        }
        if self.is_word("ON") {
            // MySQL ON DUPLICATE KEY UPDATE
            self.parse_expr_clause(block, ClauseKind::Set);
        }
        self.blocks[block].end = self.pos;
    }

    fn parse_update(&mut self) {
        let block = self.new_block(BlockKind::Update, None, self.pos);
        let kw = self.pos;
        self.pos += 1;
        self.open_clause(block, ClauseKind::UpdateTarget, kw);
        self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
        if self.is_word("TOP") {
            self.pos += 1;
            self.skip_parens(block);
        }
        if let Some(target) = self.parse_target(block) {
            self.blocks[block].sources.push(target.clone());
            self.blocks[block].target = Some(target);
        }
        if self.is_word("WITH") && self.peek_at(1).is_some_and(|t| t.punct("(")) {
            self.pos += 1;
            self.skip_parens(block);
        }
        // MySQL multi-table UPDATE: UPDATE a JOIN b ON ... SET
        while self.peek().is_some_and(|t| t.punct(",") || t.is_any(JOIN_WORDS)) {
            if self.is_punct(",") {
                self.pos += 1;
                if let Some(s) = self.parse_target(block) {
                    self.blocks[block].sources.push(s);
                }
            } else {
                let kw = self.pos;
                while self.peek().is_some_and(|t| t.is_any(JOIN_WORDS)) {
                    self.pos += 1;
                }
                self.open_clause(block, ClauseKind::JoinTable, kw);
                self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
                self.parse_table_ref(block, true);
                self.close_clause(block);
                if self.is_word("ON") {
                    let kw = self.pos;
                    self.pos += 1;
                    self.open_clause(block, ClauseKind::On, kw);
                    self.skip_expression(block, &[]);
                    self.close_clause(block);
                }
            }
        }
        self.close_clause(block);
        self.parse_dml_tail(block);
    }

    fn parse_delete(&mut self) {
        let block = self.new_block(BlockKind::Delete, None, self.pos);
        let kw = self.pos;
        self.pos += 1;
        if self.is_word("TOP") {
            self.pos += 1;
            self.skip_parens(block);
        }
        self.open_clause(block, ClauseKind::DeleteTarget, kw);
        let from_first = self.eat_word("FROM");
        self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
        if let Some(target) = self.parse_target(block) {
            self.blocks[block].sources.push(target.clone());
            self.blocks[block].target = Some(target);
        }
        let _ = from_first;
        self.close_clause(block);
        self.parse_dml_tail(block);
    }

    /// SET / FROM / WHERE / OUTPUT ... after an UPDATE or DELETE target.
    fn parse_dml_tail(&mut self, block: usize) {
        while let Some(t) = self.peek() {
            if t.punct(")") {
                return;
            }
            let u = t.upper();
            match u.as_str() {
                "SET" => self.parse_expr_clause(block, ClauseKind::Set),
                "FROM" => {
                    let kw = self.pos;
                    self.pos += 1;
                    self.open_clause(block, ClauseKind::From, kw);
                    // The first source may re-declare the target's alias; keep both (dedup by alias later).
                    self.parse_sources(block);
                    self.close_clause(block);
                }
                "WHERE" => self.parse_expr_clause(block, ClauseKind::Where),
                "ORDER" => self.parse_expr_clause(block, ClauseKind::OrderBy),
                "LIMIT" => self.parse_expr_clause(block, ClauseKind::Limit),
                "OUTPUT" | "RETURNING" => self.parse_expr_clause(block, ClauseKind::Other),
                _ => {
                    if is_statement_boundary(t) {
                        break;
                    }
                    if self.is_trailing_word() {
                        self.pos += 1;
                        continue;
                    }
                    self.parse_expr_clause(block, ClauseKind::Other);
                }
            }
        }
        self.blocks[block].end = self.pos;
    }

    fn parse_merge(&mut self) {
        let block = self.new_block(BlockKind::Merge, None, self.pos);
        let kw = self.pos;
        self.pos += 1;
        if self.is_word("TOP") {
            self.pos += 1;
            self.skip_parens(block);
        }
        self.eat_word("INTO");
        self.open_clause(block, ClauseKind::UpdateTarget, kw);
        self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
        if let Some(target) = self.parse_target(block) {
            self.blocks[block].sources.push(target.clone());
            self.blocks[block].target = Some(target);
        }
        self.close_clause(block);
        if self.is_word("USING") {
            let kw = self.pos;
            self.pos += 1;
            self.open_clause(block, ClauseKind::Using, kw);
            self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
            self.parse_table_ref(block, true);
            self.close_clause(block);
        }
        if self.is_word("ON") {
            let kw = self.pos;
            self.pos += 1;
            self.open_clause(block, ClauseKind::On, kw);
            self.skip_expression(block, &["WHEN"]);
            self.close_clause(block);
        }
        while let Some(t) = self.peek() {
            if is_statement_boundary(t) || t.punct(")") {
                break;
            }
            let u = t.upper();
            match u.as_str() {
                "SET" => self.parse_expr_clause(block, ClauseKind::Set),
                "VALUES" => {
                    let kw = self.pos;
                    self.pos += 1;
                    self.open_clause(block, ClauseKind::Values, kw);
                    if self.is_punct("(") {
                        self.skip_parens(block);
                    }
                    self.close_clause(block);
                }
                "WHEN" | "THEN" | "OUTPUT" => self.parse_expr_clause(block, ClauseKind::Other),
                "INSERT" => {
                    let kw = self.pos;
                    self.pos += 1;
                    self.open_clause(block, ClauseKind::InsertColumns, kw);
                    if self.is_punct("(") {
                        let cols = self.parse_name_list();
                        self.blocks[block].insert_columns = cols;
                    }
                    self.close_clause(block);
                }
                _ => self.parse_expr_clause(block, ClauseKind::Other),
            }
        }
        self.blocks[block].end = self.pos;
    }

    /// Anything else: collect table refs after FROM / JOIN, subqueries as children.
    fn parse_other(&mut self) {
        let block = self.new_block(BlockKind::Other, None, self.pos);
        let kw = self.pos;
        self.open_clause(block, ClauseKind::Other, kw);
        self.blocks[block].clauses.last_mut().unwrap().start = self.pos;
        while let Some(t) = self.peek() {
            if t.punct("(") {
                self.skip_parens(block);
                continue;
            }
            if t.punct(")") {
                self.pos += 1;
                continue;
            }
            if t.is("SELECT") {
                self.parse_query(Some(block));
                continue;
            }
            if t.is("FROM") || t.is_any(JOIN_WORDS) {
                let kw = self.pos;
                while self.peek().is_some_and(|t| t.is("FROM") || t.is_any(JOIN_WORDS)) {
                    self.pos += 1;
                }
                self.open_clause(block, ClauseKind::From, kw);
                self.parse_sources(block);
                self.close_clause(block);
                continue;
            }
            self.pos += 1;
        }
        self.close_clause(block);
        self.blocks[block].end = self.pos;
    }
}

fn is_statement_boundary(t: &Token) -> bool {
    t.punct(";") || t.is("GO")
}

/// Split the token stream into statements. Statement-start keywords at paren depth 0 begin a
/// new statement unless they continue the current one (`INSERT ... SELECT`, `WITH ... SELECT`,
/// `UNION SELECT`, `CREATE VIEW AS SELECT`, `IF ... BEGIN ...`).
fn split_statements(tokens: &[Token], text: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut i = 0usize;
    let mut saw_select = false;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.punct("(") {
            depth += 1;
        } else if t.punct(")") {
            depth = (depth - 1).max(0);
        } else if depth == 0 && is_statement_boundary(t) {
            if i > start {
                out.push((start, i));
            }
            start = i + 1;
            saw_select = false;
        } else if depth == 0 && t.kind == TokenKind::Word && i > start {
            let u = t.upper();
            let prev = &tokens[i - 1];
            let continues = prev.is_any(&["UNION", "ALL", "EXCEPT", "INTERSECT", "AS", "THEN", "ELSE", "BEGIN", "IS", "IN", "EXISTS", "RETURN"])
                || prev.punct("(")
                || prev.punct("=")
                || prev.punct(",")
                || prev.punct("+")
                || (u == "SELECT" && !saw_select && tokens[start].is_any(&["INSERT", "WITH", "CREATE", "ALTER", "REPLACE", "DECLARE", "SET", "IF", "WHILE", "EXPLAIN"]))
                || (u == "SET" && tokens[start].is_any(&["UPDATE", "INSERT", "MERGE"]))
                || (u == "DELETE" && tokens[start].is("MERGE"))
                || (u == "UPDATE" && (tokens[start].is("MERGE") || prev.is("KEY") || prev.is("FOR")))
                || (u == "INSERT" && tokens[start].is("MERGE"))
                || (u == "BEGIN" && tokens[start].is_any(&["IF", "WHILE", "ELSE", "CREATE", "ALTER"]))
                || u == "GO" && false;
            // A half-typed keyword at the start of a new line ("Sel|" after a finished statement)
            // is a new statement too, so completion offers SELECT rather than the old WHERE's columns.
            let typing_new_statement = i + 1 == tokens.len()
                && u.len() >= 2
                && !STATEMENT_STARTS.contains(&u.as_str())
                && STATEMENT_STARTS.iter().any(|k| k.starts_with(u.as_str()) && *k != "GO")
                && text[prev.end..t.start].contains(&'\n')
                && !prev.punct(",")
                && !prev.punct("(")
                && !prev.is_any(&["AND", "OR", "NOT", "WHERE", "ON", "SELECT", "FROM", "JOIN", "BY", "SET", "AS", "IN", "LIKE", "THEN", "ELSE", "WHEN", "CASE", "UNION", "ALL", "EXCEPT", "INTERSECT", "VALUES", "INTO", "UPDATE", "DELETE", "INSERT", "EXEC", "EXECUTE", "DECLARE", "HAVING"])
                && !matches!(prev.kind, TokenKind::Punct) ;
            if (STATEMENT_STARTS.contains(&u.as_str()) && !continues && u != "GO") || typing_new_statement {
                out.push((start, i));
                start = i;
                saw_select = false;
            }
            if u == "SELECT" {
                saw_select = true;
            }
        } else if depth == 0 && t.is("SELECT") {
            saw_select = true;
        }
        i += 1;
    }
    if start < tokens.len() {
        out.push((start, tokens.len()));
    }
    out
}

/// Batch-level declarations: `DECLARE @x type`, `CREATE TABLE #t (...)`, `SELECT ... INTO #t`.
fn collect_declarations(tokens: &[Token]) -> (Vec<Variable>, Vec<TempTable>) {
    let mut vars = Vec::new();
    let mut temps = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t.is("DECLARE") {
            // DECLARE @a int, @b varchar(10) = 'x', @t TABLE (...)
            let mut j = i + 1;
            while j < tokens.len() {
                if tokens[j].kind != TokenKind::Variable {
                    break;
                }
                let name = tokens[j].value.clone();
                j += 1;
                if j < tokens.len() && tokens[j].is("AS") {
                    j += 1;
                }
                let mut ty = String::new();
                let mut depth = 0;
                let mut cols = Vec::new();
                let is_table = j < tokens.len() && tokens[j].is("TABLE");
                while j < tokens.len() {
                    let x = &tokens[j];
                    if depth == 0 && (x.punct(",") || x.punct(";") || x.punct("=") || (x.kind == TokenKind::Word && STATEMENT_STARTS.contains(&x.upper().as_str()))) {
                        break;
                    }
                    if x.punct("(") {
                        depth += 1;
                    } else if x.punct(")") {
                        depth -= 1;
                    }
                    if is_table && depth == 1 && x.is_name() && (tokens[j - 1].punct("(") || tokens[j - 1].punct(",")) {
                        cols.push(x.value.clone());
                    }
                    if !ty.is_empty() && !x.punct("(") && !x.punct(")") && !tokens[j - 1].punct("(") {
                        ty.push(' ');
                    }
                    ty.push_str(&x.text);
                    j += 1;
                }
                if is_table {
                    temps.push(TempTable { name: name.clone(), columns: cols });
                }
                vars.push(Variable { name, data_type: ty.trim().to_string() });
                // Skip a default value expression.
                if j < tokens.len() && tokens[j].punct("=") {
                    j += 1;
                    let mut depth = 0;
                    while j < tokens.len() {
                        let x = &tokens[j];
                        if depth == 0 && (x.punct(",") || x.punct(";") || (x.kind == TokenKind::Word && STATEMENT_STARTS.contains(&x.upper().as_str()))) {
                            break;
                        }
                        if x.punct("(") {
                            depth += 1;
                        } else if x.punct(")") {
                            depth -= 1;
                        }
                        j += 1;
                    }
                }
                if j < tokens.len() && tokens[j].punct(",") {
                    j += 1;
                    continue;
                }
                break;
            }
            i = j;
            continue;
        }
        if t.is("CREATE") && i + 2 < tokens.len() && (tokens[i + 1].is("TABLE") || (tokens[i + 1].is("TEMPORARY") || tokens[i + 1].is("TEMP")) && tokens[i + 2].is("TABLE")) {
            let mut j = i + 2;
            if !tokens[i + 1].is("TABLE") {
                j += 1;
            }
            if j < tokens.len() && tokens[j].is_name() {
                let name = tokens[j].value.clone();
                j += 1;
                let mut cols = Vec::new();
                if j < tokens.len() && tokens[j].punct("(") {
                    let mut depth = 0;
                    let mut expect_col = true;
                    while j < tokens.len() {
                        let x = &tokens[j];
                        if x.punct("(") {
                            depth += 1;
                            if depth > 1 {
                                expect_col = false;
                            }
                        } else if x.punct(")") {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        } else if depth == 1 && x.punct(",") {
                            expect_col = true;
                        } else if depth == 1 && expect_col && x.is_name() {
                            if !x.is_any(&["CONSTRAINT", "PRIMARY", "UNIQUE", "FOREIGN", "CHECK", "INDEX", "KEY"]) {
                                cols.push(x.value.clone());
                            }
                            expect_col = false;
                        }
                        j += 1;
                    }
                }
                if name.starts_with('#') || tokens[i + 1].is("TEMPORARY") || tokens[i + 1].is("TEMP") {
                    temps.push(TempTable { name, columns: cols });
                }
            }
            i = j;
            continue;
        }
        if t.is("INTO") && i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::TempName && i > 0 && !tokens[i - 1].is("INSERT") {
            temps.push(TempTable { name: tokens[i + 1].value.clone(), columns: Vec::new() });
        }
        i += 1;
    }
    (vars, temps)
}

pub fn analyze(sql: &str) -> Analysis {
    let tokens = tokenize(sql);
    let text: Vec<char> = sql.chars().collect();
    let (variables, temp_tables) = collect_declarations(&tokens);
    let mut blocks: Vec<Block> = Vec::new();
    let mut statements = Vec::new();
    for (start, end) in split_statements(&tokens, &text) {
        let mut p = Parser { t: &tokens, pos: start, limit: end, blocks: std::mem::take(&mut blocks), statement_blocks: Vec::new(), ctes: Vec::new() };
        p.parse_statement();
        // Any block that never got an end gets the statement's end.
        for &b in &p.statement_blocks {
            if p.blocks[b].end <= p.blocks[b].start {
                p.blocks[b].end = end;
            }
            if let Some(c) = p.blocks[b].clauses.last_mut() {
                if c.end < c.start {
                    c.end = c.start;
                }
            }
        }
        statements.push(Statement { start, end, ctes: p.ctes, blocks: p.statement_blocks });
        blocks = p.blocks;
    }
    Analysis { tokens, statements, blocks, variables, temp_tables }
}

impl Analysis {
    /// Char span of a token range (end exclusive): start of first token to end of last.
    pub fn span(&self, start: usize, end: usize) -> (usize, usize) {
        if start >= self.tokens.len() || end <= start {
            let p = self.tokens.get(start.min(self.tokens.len().saturating_sub(1))).map(|t| t.end).unwrap_or(0);
            return (p, p);
        }
        (self.tokens[start].start, self.tokens[end - 1].end)
    }

    /// The statement the caret is in (the last one that starts at or before the caret).
    pub fn statement_at(&self, caret: usize) -> Option<usize> {
        let mut best = None;
        for (i, s) in self.statements.iter().enumerate() {
            if s.start < self.tokens.len() && self.tokens[s.start].start <= caret {
                best = Some(i);
            }
        }
        best.or(if self.statements.is_empty() { None } else { Some(0) })
    }

    /// Deepest block of the statement whose token span contains the caret. A caret just past a
    /// subquery's closing paren belongs to the parent.
    pub fn block_at(&self, statement: usize, caret: usize) -> Option<usize> {
        let st = self.statements.get(statement)?;
        let mut best: Option<usize> = None;
        for &b in &st.blocks {
            let blk = &self.blocks[b];
            let (s, e) = self.span(blk.start, blk.end);
            // Open-ended (unclosed) block: runs to the statement end.
            let closed = blk.end < self.tokens.len() && self.tokens[blk.end].punct(")") && blk.parent.is_some();
            let (st_s, st_e) = self.span(st.start, st.end);
            let _ = st_s;
            let e = if closed { e } else { e.max(st_e) };
            let in_range = caret >= s && (caret <= e || (!closed && caret >= e));
            if in_range && best.is_none_or(|cur| self.blocks[cur].depth <= blk.depth) {
                best = Some(b);
            }
        }
        best.or_else(|| st.blocks.first().copied())
    }

    /// The clause of `block` the caret is in: the last clause whose keyword starts at or before the caret.
    pub fn clause_at(&self, block: usize, caret: usize) -> Option<&Clause> {
        let blk = &self.blocks[block];
        let mut best: Option<&Clause> = None;
        for c in &blk.clauses {
            if c.keyword < self.tokens.len() && self.tokens[c.keyword].start < caret {
                best = Some(c);
            }
        }
        best
    }

    /// Sources visible from `block`: its own plus every ancestor's (correlated references).
    pub fn visible_sources(&self, block: usize) -> Vec<&Source> {
        let mut out = Vec::new();
        let mut cur = Some(block);
        while let Some(b) = cur {
            // Later definitions first, so `UPDATE u ... FROM Users u` resolves `u` to Users.
            for s in self.blocks[b].sources.iter().rev() {
                out.push(s);
            }
            cur = self.blocks[b].parent;
        }
        out
    }

    pub fn cte(&self, statement: usize, name: &str) -> Option<&Cte> {
        self.statements.get(statement)?.ctes.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    pub fn temp_table(&self, name: &str) -> Option<&TempTable> {
        self.temp_tables.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// Token index of the last token that ends at or before the caret.
    pub fn token_before(&self, caret: usize) -> Option<usize> {
        let mut best = None;
        for (i, t) in self.tokens.iter().enumerate() {
            if t.end <= caret {
                best = Some(i);
            } else {
                break;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(sql: &str, block: usize) -> Vec<(String, Option<String>)> {
        let a = analyze(sql);
        a.blocks[block].sources.iter().map(|s| (s.table_name().unwrap_or_else(|| format!("{:?}", s.kind)), s.alias.clone())).collect()
    }

    #[test]
    fn simple_from_and_joins() {
        let s = sources("SELECT o.Id FROM dbo.Orders o INNER JOIN Customers AS c ON o.CustomerId = c.Id LEFT JOIN Items i ON 1=1 WHERE 1=1", 0);
        assert_eq!(s, vec![("dbo.Orders".into(), Some("o".into())), ("Customers".into(), Some("c".into())), ("Items".into(), Some("i".into()))]);
    }

    #[test]
    fn comma_list_and_quoted_names() {
        let s = sources("SELECT * FROM [dbo].[Order Items] oi, `b` c, \"d\"", 0);
        assert_eq!(s[0], ("dbo.Order Items".into(), Some("oi".into())));
        assert_eq!(s[1], ("b".into(), Some("c".into())));
        assert_eq!(s[2], ("d".into(), None));
    }

    #[test]
    fn cte_and_derived_tables() {
        let a = analyze("WITH recent AS (SELECT Id, Total FROM Orders WHERE X = 1) SELECT r.Id FROM recent r JOIN (SELECT a, b AS bb FROM T) d ON d.a = r.Id");
        assert_eq!(a.statements.len(), 1);
        let st = &a.statements[0];
        assert_eq!(st.ctes.len(), 1);
        assert_eq!(st.ctes[0].name, "recent");
        let cte_block = &a.blocks[st.ctes[0].block.unwrap()];
        assert_eq!(cte_block.projection.iter().map(|p| p.name.clone().unwrap()).collect::<Vec<_>>(), ["Id", "Total"]);
        // The main SELECT block: first block created at statement level after the WITH block.
        let main = st.blocks.iter().copied().find(|&b| a.blocks[b].parent.is_none() && a.blocks[b].kind == BlockKind::Select).unwrap();
        let mb = &a.blocks[main];
        assert_eq!(mb.sources.len(), 2);
        assert_eq!(mb.sources[0].kind, SourceKind::Cte("recent".into()));
        assert!(matches!(mb.sources[1].kind, SourceKind::Derived(_)));
        assert_eq!(mb.sources[1].alias.as_deref(), Some("d"));
        if let SourceKind::Derived(db) = mb.sources[1].kind {
            assert_eq!(a.blocks[db].projection.iter().map(|p| p.name.clone().unwrap()).collect::<Vec<_>>(), ["a", "bb"]);
        }
    }

    #[test]
    fn subquery_in_where_is_a_child_block() {
        let a = analyze("SELECT * FROM Orders o WHERE o.CustomerId IN (SELECT Id FROM Customers WHERE Name LIKE 'a%')");
        assert_eq!(a.blocks.len(), 2);
        let child = &a.blocks[1];
        assert_eq!(child.parent, Some(0));
        assert_eq!(child.sources[0].table_name().unwrap(), "Customers");
        let caret = "SELECT * FROM Orders o WHERE o.CustomerId IN (SELECT Id FROM Customers WHERE ".len();
        let b = a.block_at(0, caret).unwrap();
        assert_eq!(b, 1);
        assert_eq!(a.clause_at(b, caret).unwrap().kind, ClauseKind::Where);
        let vis = a.visible_sources(b);
        assert_eq!(vis.len(), 2);
        // After the closing paren we are back in the outer block.
        let a2 = analyze("SELECT * FROM Orders o WHERE o.CustomerId IN (SELECT Id FROM Customers) AND ");
        let caret2 = a2.tokens.last().unwrap().end + 1;
        assert_eq!(a2.block_at(0, caret2).unwrap(), 0);
    }

    #[test]
    fn update_from_and_delete() {
        let a = analyze("UPDATE u SET u.Name = 'x' FROM Users u JOIN Roles r ON r.Id = u.RoleId WHERE r.Name = 'a'");
        let b = &a.blocks[0];
        assert_eq!(b.kind, BlockKind::Update);
        assert_eq!(b.target.as_ref().unwrap().table_name().unwrap(), "u");
        assert!(b.sources.iter().any(|s| s.table_name().as_deref() == Some("Users") && s.alias.as_deref() == Some("u")));
        assert!(b.sources.iter().any(|s| s.table_name().as_deref() == Some("Roles")));
        assert!(b.clauses.iter().any(|c| c.kind == ClauseKind::Set));
        let d = analyze("DELETE FROM dbo.Log WHERE Created < '2020'");
        assert_eq!(d.blocks[0].kind, BlockKind::Delete);
        assert_eq!(d.blocks[0].target.as_ref().unwrap().table_name().unwrap(), "dbo.Log");
    }

    #[test]
    fn insert_forms() {
        let a = analyze("INSERT INTO dbo.T (a, b) VALUES (1, 2)");
        assert_eq!(a.blocks[0].insert_columns, ["a", "b"]);
        assert!(a.blocks[0].clauses.iter().any(|c| c.kind == ClauseKind::Values));
        let b = analyze("INSERT INTO T (a) SELECT x FROM S");
        assert_eq!(b.statements.len(), 1);
        assert_eq!(b.blocks.len(), 2);
        assert_eq!(b.blocks[1].parent, Some(0));
    }

    #[test]
    fn statements_split_on_semicolons_keywords_and_go() {
        let a = analyze("SELECT 1 FROM A; SELECT 2 FROM B\nGO\nSELECT 3 FROM C SELECT 4 FROM D");
        assert_eq!(a.statements.len(), 4);
        let u = analyze("SELECT 1 FROM A UNION ALL SELECT 2 FROM B");
        assert_eq!(u.statements.len(), 1);
    }

    #[test]
    fn half_typed_keyword_on_a_new_line_starts_a_statement() {
        let sql = "SELECT * FROM Staff WHERE StaffId = 'x'\n\nSel";
        let a = analyze(sql);
        assert_eq!(a.statements.len(), 2);
        // Same line: still part of the WHERE expression.
        let a = analyze("SELECT * FROM Staff WHERE StaffId = Sel");
        assert_eq!(a.statements.len(), 1);
        // A value expected after an operator or AND stays in the expression.
        let a = analyze("SELECT * FROM Staff WHERE StaffId =\nSel");
        assert_eq!(a.statements.len(), 1);
        let a = analyze("SELECT * FROM Staff WHERE StaffId = 1 AND\nSel");
        assert_eq!(a.statements.len(), 1);
    }

    #[test]
    fn declarations_and_temp_tables() {
        let a = analyze("DECLARE @id INT = 5, @name NVARCHAR(50), @t TABLE (x INT, y INT);\nCREATE TABLE #tmp (Id INT PRIMARY KEY, Name VARCHAR(10));\nSELECT * INTO #copy FROM T;\nSELECT * FROM #tmp t WHERE t.Id = @id");
        assert_eq!(a.variables.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["@id", "@name", "@t"]);
        assert_eq!(a.variables[1].data_type, "NVARCHAR(50)");
        assert_eq!(a.temp_table("#tmp").unwrap().columns, ["Id", "Name"]);
        assert_eq!(a.temp_table("@t").unwrap().columns, ["x", "y"]);
        assert!(a.temp_table("#copy").is_some());
        let last = a.statements.last().unwrap();
        let b = &a.blocks[last.blocks[0]];
        assert_eq!(b.sources[0].kind, SourceKind::Temp("#tmp".into()));
    }

    #[test]
    fn projection_names_and_stars() {
        let a = analyze("SELECT t.*, a, b AS bb, c cc, COUNT(*) AS n, (SELECT 1) sub, x.y.z FROM T t");
        let p = &a.blocks[0].projection;
        assert!(p[0].star && p[0].star_prefix.as_deref() == Some("t"));
        assert_eq!(p[1].name.as_deref(), Some("a"));
        assert_eq!(p[2].name.as_deref(), Some("bb"));
        assert_eq!(p[3].name.as_deref(), Some("cc"));
        assert_eq!(p[4].name.as_deref(), Some("n"));
        assert_eq!(p[5].name.as_deref(), Some("sub"));
        assert_eq!(p[6].name.as_deref(), Some("z"));
    }

    #[test]
    fn half_typed_statements_do_not_panic() {
        for sql in ["SELECT", "SELECT * FROM", "SELECT * FROM (", "SELECT * FROM (SELECT", "WITH x AS (", "INSERT INTO", "UPDATE", "DELETE FROM t WHERE (", "SELECT 'unterminated", "FROM", ")))"] {
            let a = analyze(sql);
            let _ = a.statement_at(sql.len()).map(|s| a.block_at(s, sql.len()));
        }
    }

    #[test]
    fn join_table_clause_is_detected() {
        let sql = "SELECT * FROM A a LEFT JOIN ";
        let a = analyze(sql);
        let b = a.block_at(0, sql.len()).unwrap();
        assert_eq!(a.clause_at(b, sql.len()).unwrap().kind, ClauseKind::JoinTable);
        let sql2 = "SELECT * FROM A a LEFT JOIN B b ON ";
        let a2 = analyze(sql2);
        assert_eq!(a2.clause_at(0, sql2.len()).unwrap().kind, ClauseKind::On);
    }
}
