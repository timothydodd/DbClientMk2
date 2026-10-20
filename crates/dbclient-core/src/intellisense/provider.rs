//! Turns a [`SqlContext`] into ranked completions: tables / views / CTEs / temp tables where a
//! table is expected, columns of the sources in scope (CTEs and derived tables resolved through
//! their projections), aliases, variables, routines, dialect keywords, snippets, plus actions
//! (`*` expansion, JOIN clauses and ON conditions from foreign keys, INSERT column templates).
//! Also hover information and lightweight diagnostics for the editor.

use super::parser;
use super::scope::{Analysis, Source, SourceKind};
use super::snippets;
use super::{
    CompletionItem, CompletionType, Diagnostic, HoverInfo, SchemaColumn, SchemaForeignKey, SchemaProvider, SchemaRoutine, SchemaTable,
    SqlContext, SqlContextType,
};
use crate::models::DbDatabase;
use crate::sql_identifier::quote;
use crate::SqlDialect;
use std::collections::HashMap;

/// Schema provider over a loaded database: tables and views, with columns cached under both
/// the bare name and `schema.name` so schema-qualified references resolve.
pub struct StaticSchema {
    tables: Vec<SchemaTable>,
    columns: HashMap<String, Vec<SchemaColumn>>,
    keywords: Vec<String>,
    default_schema: Option<String>,
    foreign_keys: Vec<SchemaForeignKey>,
    routines: Vec<SchemaRoutine>,
    dialect: SqlDialect,
}

impl StaticSchema {
    pub fn from_database(db: &DbDatabase, keywords: Vec<String>, default_schema: Option<String>, dialect: SqlDialect) -> Self {
        let mut tables = Vec::new();
        let mut columns: HashMap<String, Vec<SchemaColumn>> = HashMap::new();
        let mut cache = |name: &str, schema: &str, cols: Vec<SchemaColumn>| {
            // On a bare-name collision across schemas the first table wins; the qualified key stays exact.
            columns.entry(name.to_lowercase()).or_insert_with(|| cols.clone());
            if !schema.is_empty() {
                columns.insert(format!("{}.{}", schema, name).to_lowercase(), cols);
            }
        };
        for (t, is_view) in db.tables.iter().map(|t| (t, false)).chain(db.views.iter().map(|v| (v, true))) {
            tables.push(SchemaTable { name: t.name.clone(), schema: (!t.schema.is_empty()).then(|| t.schema.clone()), is_view });
            let cols = t
                .columns
                .iter()
                .map(|c| SchemaColumn { name: c.name.clone(), data_type: c.data_type.clone(), is_nullable: c.is_nullable, is_primary_key: c.is_primary_key })
                .collect();
            cache(&t.name, &t.schema, cols);
        }
        let foreign_keys = db
            .foreign_keys
            .iter()
            .map(|f| SchemaForeignKey {
                schema: (!f.schema.is_empty()).then(|| f.schema.clone()),
                table: f.table.clone(),
                columns: f.columns.clone(),
                ref_schema: (!f.ref_schema.is_empty()).then(|| f.ref_schema.clone()),
                ref_table: f.ref_table.clone(),
                ref_columns: f.ref_columns.clone(),
            })
            .collect();
        let routines = db
            .stored_procedures
            .iter()
            .map(|p| (p, false))
            .chain(db.functions.iter().map(|f| (f, true)))
            .map(|(p, is_function)| SchemaRoutine {
                name: p.name.clone(),
                schema: (!p.schema.is_empty()).then(|| p.schema.clone()),
                is_function,
                parameters: p.parameters.iter().map(|x| (x.name.clone(), x.data_type.clone())).collect(),
                returns: p.returns.clone(),
            })
            .collect();
        StaticSchema { tables, columns, keywords, default_schema, foreign_keys, routines, dialect }
    }
}

impl SchemaProvider for StaticSchema {
    fn tables(&self) -> Vec<SchemaTable> {
        self.tables.clone()
    }
    fn columns(&self, table_name: &str) -> Vec<SchemaColumn> {
        if let Some(c) = self.columns.get(&table_name.to_lowercase()) {
            return c.clone();
        }
        // `db.schema.table` → `schema.table`; a prefix that isn't a schema → bare name.
        let parts: Vec<&str> = table_name.split('.').collect();
        if parts.len() >= 2 {
            let two = format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]).to_lowercase();
            if let Some(c) = self.columns.get(&two) {
                return c.clone();
            }
            if let Some(c) = self.columns.get(&parts[parts.len() - 1].to_lowercase()) {
                return c.clone();
            }
        }
        Vec::new()
    }
    fn keywords(&self) -> Vec<String> {
        self.keywords.clone()
    }
    fn default_schema(&self) -> Option<String> {
        self.default_schema.clone()
    }
    fn foreign_keys(&self) -> Vec<SchemaForeignKey> {
        self.foreign_keys.clone()
    }
    fn routines(&self) -> Vec<SchemaRoutine> {
        self.routines.clone()
    }
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }
}

pub struct IntelliSenseProvider {
    schema: Box<dyn SchemaProvider + Send + Sync>,
    tables: Vec<SchemaTable>,
    keywords: Vec<String>,
    default_schema: Option<String>,
    foreign_keys: Vec<SchemaForeignKey>,
    routines: Vec<SchemaRoutine>,
    dialect: SqlDialect,
}

/// A resolved column with the source it came from.
#[derive(Debug, Clone)]
struct ScopedColumn {
    column: SchemaColumn,
    /// How the source is referenced (`o`, `Orders`), for qualifying.
    source_ref: String,
}

impl IntelliSenseProvider {
    pub fn new(schema: Box<dyn SchemaProvider + Send + Sync>) -> Self {
        let tables = schema.tables();
        let keywords = schema.keywords();
        let default_schema = schema.default_schema();
        let foreign_keys = schema.foreign_keys();
        let routines = schema.routines();
        let dialect = schema.dialect();
        Self { schema, tables, keywords, default_schema, foreign_keys, routines, dialect }
    }

    // --- resolution ----------------------------------------------------------------

    fn find_table(&self, name: &str) -> Option<&SchemaTable> {
        let parts: Vec<&str> = name.split('.').collect();
        let (schema, bare) = if parts.len() >= 2 { (Some(parts[parts.len() - 2]), parts[parts.len() - 1]) } else { (None, parts[0]) };
        let mut candidates = self.tables.iter().filter(|t| t.name.eq_ignore_ascii_case(bare));
        match schema {
            Some(s) => candidates.find(|t| t.schema.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(s))),
            None => {
                let all: Vec<&SchemaTable> = candidates.collect();
                all.iter()
                    .find(|t| t.schema.as_deref().is_none() || t.schema.as_deref() == self.default_schema.as_deref())
                    .copied()
                    .or_else(|| all.first().copied())
            }
        }
    }

    fn is_schema_name(&self, name: &str) -> bool {
        self.tables.iter().any(|t| t.schema.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(name)))
    }

    fn source_label(&self, src: &Source, analysis: &Analysis, statement: Option<usize>) -> String {
        match &src.kind {
            SourceKind::Table(p) => p.join("."),
            SourceKind::Cte(n) => format!("{n} (CTE)"),
            SourceKind::Derived(_) => format!("{} (subquery)", src.alias.clone().unwrap_or_default()),
            SourceKind::Temp(n) => n.clone(),
            SourceKind::Function(p) => format!("{}()", p.join(".")),
            SourceKind::Other => src.alias.clone().unwrap_or_default(),
        }
        .to_string()
            + &{
                let _ = (analysis, statement);
                String::new()
            }
    }

    /// Columns of one source, following CTEs, derived tables and temp tables.
    fn source_columns(&self, analysis: &Analysis, statement: Option<usize>, src: &Source, depth: usize) -> Vec<SchemaColumn> {
        if !src.columns.is_empty() {
            return src.columns.iter().map(|c| SchemaColumn { name: c.clone(), ..Default::default() }).collect();
        }
        if depth > 8 {
            return Vec::new();
        }
        match &src.kind {
            SourceKind::Table(parts) => self.schema.columns(&parts.join(".")),
            SourceKind::Cte(name) => {
                let Some(st) = statement else { return Vec::new() };
                let Some(cte) = analysis.cte(st, name) else { return Vec::new() };
                if !cte.columns.is_empty() {
                    return cte.columns.iter().map(|c| SchemaColumn { name: c.clone(), ..Default::default() }).collect();
                }
                cte.block.map(|b| self.block_columns(analysis, statement, b, depth + 1)).unwrap_or_default()
            }
            SourceKind::Derived(b) => self.block_columns(analysis, statement, *b, depth + 1),
            SourceKind::Temp(name) => analysis
                .temp_table(name)
                .map(|t| t.columns.iter().map(|c| SchemaColumn { name: c.clone(), ..Default::default() }).collect())
                .unwrap_or_default(),
            SourceKind::Function(_) | SourceKind::Other => Vec::new(),
        }
    }

    /// Output columns of a SELECT block (its projection, stars expanded through its sources).
    fn block_columns(&self, analysis: &Analysis, statement: Option<usize>, block: usize, depth: usize) -> Vec<SchemaColumn> {
        let blk = &analysis.blocks[block];
        let mut out = Vec::new();
        for p in &blk.projection {
            if p.star {
                for src in &blk.sources {
                    if let Some(prefix) = &p.star_prefix {
                        if !src.reference().is_some_and(|r| r.eq_ignore_ascii_case(prefix)) {
                            continue;
                        }
                    }
                    out.extend(self.source_columns(analysis, statement, src, depth + 1));
                }
            } else if let Some(name) = &p.name {
                // Carry the type through when the name is a column of one of the sources.
                let typed = blk.sources.iter().flat_map(|s| self.source_columns(analysis, statement, s, depth + 1)).find(|c| c.name.eq_ignore_ascii_case(name));
                out.push(typed.unwrap_or_else(|| SchemaColumn { name: name.clone(), ..Default::default() }));
            }
        }
        out
    }

    fn find_source<'a>(&self, ctx: &'a SqlContext, reference: &str) -> Option<&'a Source> {
        let b = ctx.block?;
        ctx.analysis.visible_sources(b).into_iter().find(|s| s.reference().is_some_and(|r| r.eq_ignore_ascii_case(reference)))
    }

    /// All columns visible from the context's block, with their source.
    fn scoped_columns(&self, ctx: &SqlContext) -> Vec<ScopedColumn> {
        let Some(b) = ctx.block else { return Vec::new() };
        let mut out = Vec::new();
        for src in ctx.analysis.visible_sources(b) {
            let Some(r) = src.reference() else { continue };
            for c in self.source_columns(&ctx.analysis, ctx.statement, src, 0) {
                out.push(ScopedColumn { column: c, source_ref: r.to_string() });
            }
        }
        out
    }

    // --- completions -----------------------------------------------------------------

    pub fn completions(&self, sql: &str, caret: usize, recent: &[String]) -> Vec<CompletionItem> {
        let ctx = parser::analyze_context(sql, caret);
        let partial = ctx.partial.clone();
        let mut items: Vec<CompletionItem> = Vec::new();
        let kind = ctx.context_type();

        if ctx.in_string.is_some() {
            // Only values make sense inside quotes.
            self.add_literals(&mut items, &ctx, 0);
            return self.finish(items, &partial, recent);
        }
        if self.after_operator(&ctx) {
            self.add_literals(&mut items, &ctx, 0);
        }

        match kind {
            SqlContextType::ColumnAfterDot => self.add_after_dot(&mut items, &ctx),
            SqlContextType::FromClause | SqlContextType::InsertInto | SqlContextType::UpdateTable => {
                if kind == SqlContextType::InsertInto && ctx.after_insert_target {
                    self.add_insert_template(&mut items, &ctx);
                    self.add_keywords_filtered(&mut items, &["VALUES", "SELECT", "DEFAULT VALUES", "OUTPUT"], 1);
                } else {
                    if ctx.last_keyword == "JOIN" {
                        self.add_fk_joins(&mut items, &ctx, false);
                    }
                    self.add_tables(&mut items, &ctx, 2);
                    self.add_keywords(&mut items, 6);
                }
            }
            SqlContextType::SelectList => {
                if let Some(star_from) = ctx.after_star {
                    self.add_star_expansion(&mut items, &ctx, star_from);
                }
                self.add_columns(&mut items, &ctx, 1);
                self.add_aliases(&mut items, &ctx, 2);
                self.add_variables(&mut items, &ctx, 3);
                self.add_functions(&mut items, 4);
                // The clause keywords that can follow a select list come before function names.
                self.add_keywords_filtered(&mut items, &["FROM", "AS", "DISTINCT", "TOP", "INTO"], 0);
                self.add_keywords(&mut items, 5);
            }
            SqlContextType::WhereClause | SqlContextType::JoinCondition => {
                if kind == SqlContextType::JoinCondition {
                    self.add_fk_conditions(&mut items, &ctx);
                }
                self.add_columns(&mut items, &ctx, 1);
                self.add_aliases(&mut items, &ctx, 2);
                self.add_variables(&mut items, &ctx, 3);
                self.add_functions(&mut items, 4);
                self.add_keywords_filtered(&mut items, &["AND", "OR", "NOT", "IN", "IS NULL", "IS NOT NULL", "LIKE", "BETWEEN", "EXISTS", "ORDER BY", "GROUP BY", "HAVING"], 0);
                self.add_keywords(&mut items, 5);
            }
            SqlContextType::UpdateSet => {
                self.add_target_columns(&mut items, &ctx, 0);
                self.add_columns(&mut items, &ctx, 1);
                self.add_aliases(&mut items, &ctx, 2);
                self.add_variables(&mut items, &ctx, 3);
                self.add_functions(&mut items, 4);
                self.add_keywords_filtered(&mut items, &["FROM", "WHERE", "OUTPUT", "NULL", "DEFAULT"], 5);
            }
            SqlContextType::InsertColumns => self.add_target_columns(&mut items, &ctx, 0),
            SqlContextType::InsertValues => {
                self.add_variables(&mut items, &ctx, 1);
                self.add_functions(&mut items, 2);
                self.add_keywords_filtered(&mut items, &["NULL", "DEFAULT", "GETDATE()", "NOW()", "SYSDATETIME()", "NEWID()", "CURRENT_TIMESTAMP"], 3);
            }
            SqlContextType::AfterGroupOrOrder => items.push(CompletionItem::new("BY", CompletionType::Keyword, 0, None)),
            SqlContextType::AfterTableName | SqlContextType::AfterTableAlias => {
                self.add_fk_joins(&mut items, &ctx, true);
                for (k, p) in [
                    ("JOIN", 1),
                    ("INNER JOIN", 1),
                    ("LEFT JOIN", 1),
                    ("RIGHT JOIN", 1),
                    ("CROSS JOIN", 1),
                    ("WHERE", 2),
                    ("ORDER BY", 3),
                    ("GROUP BY", 3),
                    ("HAVING", 4),
                    ("UNION", 4),
                    ("AS", 4),
                ] {
                    items.push(CompletionItem::new(k, CompletionType::Keyword, p, None));
                }
                if self.dialect == SqlDialect::SqlServer {
                    items.push(CompletionItem::new("WITH (NOLOCK)", CompletionType::Keyword, 5, None));
                    items.push(CompletionItem::new("SET", CompletionType::Keyword, 2, None));
                    items.push(CompletionItem::new("CROSS APPLY", CompletionType::Keyword, 2, None));
                    items.push(CompletionItem::new("OUTER APPLY", CompletionType::Keyword, 2, None));
                } else {
                    items.push(CompletionItem::new("SET", CompletionType::Keyword, 2, None));
                    items.push(CompletionItem::new("LIMIT", CompletionType::Keyword, 3, None));
                }
            }
            SqlContextType::General | SqlContextType::Unknown => {
                let prev_exec = self.prev_token_is(&ctx, &["EXEC", "EXECUTE", "CALL"]);
                if prev_exec {
                    self.add_procedures(&mut items, 0);
                } else {
                    self.add_snippets(&mut items, &partial);
                    self.add_variables(&mut items, &ctx, 2);
                    self.add_keywords(&mut items, 3);
                    self.add_procedures(&mut items, 6);
                }
            }
        }

        self.finish(items, &partial, recent)
    }

    fn finish(&self, items: Vec<CompletionItem>, partial: &str, recent: &[String]) -> Vec<CompletionItem> {
        // Filter and rank.
        let mut scored: Vec<CompletionItem> = Vec::new();
        for mut item in items {
            let Some(score) = match_score(&partial, &item.text) else { continue };
            item.score = score;
            scored.push(item);
        }
        // Dedupe by inserted text + kind, keeping the best.
        scored.sort_by(|a, b| rank_key(a, recent).cmp(&rank_key(b, recent)));
        let mut seen: HashMap<String, ()> = HashMap::new();
        scored.retain(|i| seen.insert(format!("{:?}:{}", i.kind, i.insert_text.as_deref().unwrap_or(&i.text).to_lowercase()), ()).is_none());
        scored
    }

    /// The caret follows a comparison / membership operator, where a value is expected.
    fn after_operator(&self, ctx: &SqlContext) -> bool {
        let Some(i) = ctx.analysis.token_before(ctx.word_start) else { return false };
        let t = &ctx.analysis.tokens[i];
        t.is_any(&["LIKE", "IN", "BETWEEN", "AND"]) && i > 0 && !ctx.analysis.tokens[i - 1].is("BETWEEN") && !t.is("AND")
            || t.punct("=") || t.punct("<>") || t.punct("!=") || t.punct(">") || t.punct("<") || t.punct(">=") || t.punct("<=")
            || (t.punct("(") && i > 0 && ctx.analysis.tokens[i - 1].is_any(&["IN", "VALUES"]))
            || (t.punct(",") && ctx.in_string.is_none() && matches!(ctx.clause, Some(super::scope::ClauseKind::Values)))
    }

    /// Literal values used elsewhere in the editor (strings, numbers), newest-looking first.
    fn add_literals(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, priority: i32) {
        let caret = ctx.word_start;
        let mut seen: Vec<String> = Vec::new();
        for t in ctx.analysis.tokens.iter().rev() {
            let (value, quoted) = match t.kind {
                super::lexer::TokenKind::String => {
                    let body = t.text.trim_start_matches(['N', 'n']);
                    let inner = body.trim_start_matches('\'').trim_end_matches('\'').replace("''", "'");
                    (inner, true)
                }
                super::lexer::TokenKind::Number => (t.value.clone(), false),
                _ => continue,
            };
            // Skip the literal being typed and trivial values.
            if t.start <= caret && caret <= t.end || value.is_empty() || value.chars().count() > 200 || value.chars().all(|c| c.is_ascii_digit()) && value.len() < 2 {
                continue;
            }
            if seen.iter().any(|v| v.eq_ignore_ascii_case(&value)) {
                continue;
            }
            seen.push(value.clone());
            let insert = if ctx.in_string.is_some() || !quoted { if quoted { format!("'{}'", value.replace('\'', "''")) } else { value.clone() } } else { format!("'{}'", value.replace('\'', "''")) };
            let mut item = CompletionItem::new(value, CompletionType::Value, priority, Some(if quoted { "value used in this editor" } else { "number used in this editor" }.into())).with_insert(insert);
            item.replace_from = Some(ctx.in_string.unwrap_or(ctx.word_start));
            items.push(item);
            if seen.len() >= 50 {
                break;
            }
        }
    }

    fn prev_token_is(&self, ctx: &SqlContext, words: &[&str]) -> bool {
        let caret = ctx.word_start;
        let Some(i) = ctx.analysis.token_before(caret) else { return false };
        ctx.analysis.tokens[i].is_any(words)
    }

    fn add_after_dot(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext) {
        let prefix = ctx.table_prefix.as_str();
        // 1. A visible source (alias or bare name).
        if let Some(src) = self.find_source(ctx, prefix) {
            let cols = self.source_columns(&ctx.analysis, ctx.statement, src, 0);
            for c in cols {
                items.push(column_item(&c, None, if c.is_primary_key { 0 } else { 1 }));
            }
            if !items.is_empty() {
                items.push(CompletionItem::new("*", CompletionType::Column, 2, Some("all columns".into())));
                return;
            }
        }
        // 2. A temp table / table variable.
        if let Some(t) = ctx.analysis.temp_table(prefix) {
            for c in &t.columns {
                items.push(CompletionItem::new(c.clone(), CompletionType::Column, 1, None));
            }
            if !items.is_empty() {
                return;
            }
        }
        // 3. A schema: its tables, views and routines.
        if !prefix.contains('.') && self.is_schema_name(prefix) {
            for t in &self.tables {
                if t.schema.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(prefix)) {
                    items.push(table_item(t, t.name.clone(), 1));
                }
            }
            for r in &self.routines {
                if r.schema.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(prefix)) {
                    items.push(routine_item(r, 3));
                }
            }
            return;
        }
        // 4. A table named directly (`Orders.`, `dbo.Orders.`).
        let cols = self.schema.columns(prefix);
        for c in cols {
            items.push(column_item(&c, None, if c.is_primary_key { 0 } else { 1 }));
        }
    }

    fn add_tables(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, priority: i32) {
        // Names shared by several schemas must be inserted schema-qualified to be unambiguous.
        let mut ambiguous: Vec<String> = Vec::new();
        for t in &self.tables {
            let mut schemas: Vec<String> = self.tables.iter().filter(|o| o.name.eq_ignore_ascii_case(&t.name)).map(|o| o.schema.clone().unwrap_or_default().to_lowercase()).collect();
            schemas.sort();
            schemas.dedup();
            if schemas.len() > 1 {
                ambiguous.push(t.name.to_lowercase());
            }
        }
        let mut schemas: Vec<String> = Vec::new();
        for t in &self.tables {
            let schema = t.schema.clone().unwrap_or_default();
            let needs_qualifying = !schema.is_empty()
                && (ambiguous.contains(&t.name.to_lowercase()) || !self.default_schema.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(&schema)));
            let text = if needs_qualifying { format!("{}.{}", schema, t.name) } else { t.name.clone() };
            items.push(table_item(t, text, priority));
            if !schema.is_empty() && !schemas.iter().any(|s| s.eq_ignore_ascii_case(&schema)) {
                schemas.push(schema);
            }
        }
        for s in schemas {
            items.push(CompletionItem::new(s, CompletionType::Schema, priority + 1, Some("Schema".into())));
        }
        if let Some(st) = ctx.statement {
            for cte in &ctx.analysis.statements[st].ctes {
                items.push(CompletionItem::new(cte.name.clone(), CompletionType::Cte, priority - 1, Some("CTE".into())));
            }
        }
        for t in &ctx.analysis.temp_tables {
            items.push(CompletionItem::new(t.name.clone(), CompletionType::Table, priority - 1, Some("temp table".into())));
        }
        for r in self.routines.iter().filter(|r| r.is_function && r.returns.to_uppercase().contains("TABLE")) {
            items.push(routine_item(r, priority + 2));
        }
    }

    fn add_columns(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, base_priority: i32) {
        // Merge same-named columns from different sources into one item listing every source;
        // when a name is ambiguous, also offer the qualified forms.
        let scoped = self.scoped_columns(ctx);
        let mut groups: Vec<(SchemaColumn, Vec<ScopedColumn>)> = Vec::new();
        for sc in scoped {
            match groups.iter_mut().find(|(c, _)| c.name.eq_ignore_ascii_case(&sc.column.name)) {
                Some((_, v)) => v.push(sc),
                None => groups.push((sc.column.clone(), vec![sc])),
            }
        }
        for (col, sources) in groups {
            let desc = format!(
                "{}{}",
                sources.iter().map(|s| format!("{}.{}", s.source_ref, s.column.name)).collect::<Vec<_>>().join(", "),
                if col.data_type.is_empty() { String::new() } else { format!(" ({})", col.data_type) }
            );
            items.push(CompletionItem::new(col.name.clone(), CompletionType::Column, base_priority, Some(desc)));
            if sources.len() > 1 {
                for s in &sources {
                    items.push(column_item(&s.column, Some(&s.source_ref), base_priority));
                }
            }
        }
    }

    fn add_target_columns(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, priority: i32) {
        let Some(b) = ctx.block else { return };
        let blk = &ctx.analysis.blocks[b];
        let Some(target) = &blk.target else { return };
        // UPDATE alias SET ... FROM table alias: the target may be an alias defined in FROM.
        let src = target.reference().and_then(|r| self.find_source(ctx, r)).unwrap_or(target);
        let listed: Vec<String> = blk.insert_columns.iter().map(|c| c.to_lowercase()).collect();
        for c in self.source_columns(&ctx.analysis, ctx.statement, src, 0) {
            if listed.contains(&c.name.to_lowercase()) {
                continue;
            }
            items.push(column_item(&c, None, priority));
        }
    }

    fn add_aliases(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, priority: i32) {
        let Some(b) = ctx.block else { return };
        for src in ctx.analysis.visible_sources(b) {
            if let Some(r) = src.reference() {
                items.push(CompletionItem::new(r, CompletionType::Alias, priority, Some(self.source_label(src, &ctx.analysis, ctx.statement))));
            }
        }
    }

    fn add_variables(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, priority: i32) {
        for v in &ctx.analysis.variables {
            items.push(CompletionItem::new(v.name.clone(), CompletionType::Variable, priority, Some(v.data_type.clone())));
        }
    }

    fn add_functions(&self, items: &mut Vec<CompletionItem>, priority: i32) {
        for r in self.routines.iter().filter(|r| r.is_function) {
            items.push(routine_item(r, priority));
        }
    }

    fn add_procedures(&self, items: &mut Vec<CompletionItem>, priority: i32) {
        for r in self.routines.iter().filter(|r| !r.is_function) {
            items.push(routine_item(r, priority));
        }
    }

    fn add_keywords(&self, items: &mut Vec<CompletionItem>, priority: i32) {
        for k in &self.keywords {
            items.push(CompletionItem::new(k.clone(), CompletionType::Keyword, priority, None));
        }
    }

    fn add_keywords_filtered(&self, items: &mut Vec<CompletionItem>, words: &[&str], priority: i32) {
        for w in words {
            items.push(CompletionItem::new(*w, CompletionType::Keyword, priority, None));
        }
    }

    fn add_snippets(&self, items: &mut Vec<CompletionItem>, partial: &str) {
        for s in snippets::for_dialect(self.dialect) {
            // Snippets show up when their shortcut (or label) is being typed; never on an empty prefix.
            if partial.is_empty() {
                continue;
            }
            items.push(CompletionItem::new(s.shortcut, CompletionType::Snippet, 0, Some(s.label.into())).with_insert(s.body));
        }
    }

    /// `SELECT *|` → one item that replaces the star with the full column list.
    fn add_star_expansion(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, star_from: usize) {
        let Some(b) = ctx.block else { return };
        let blk = &ctx.analysis.blocks[b];
        let sources: Vec<&Source> = if ctx.table_prefix.is_empty() {
            blk.sources.iter().collect()
        } else {
            blk.sources.iter().filter(|s| s.reference().is_some_and(|r| r.eq_ignore_ascii_case(&ctx.table_prefix))).collect()
        };
        if sources.is_empty() {
            return;
        }
        let qualify = blk.sources.len() > 1;
        let mut names = Vec::new();
        for src in &sources {
            let r = src.reference().unwrap_or("");
            for c in self.source_columns(&ctx.analysis, ctx.statement, src, 0) {
                names.push(if qualify { format!("{r}.{}", self.quote_if_needed(&c.name)) } else { self.quote_if_needed(&c.name) });
            }
        }
        if names.is_empty() {
            return;
        }
        let text = names.join(", ");
        let mut item = CompletionItem::new("Expand * to columns", CompletionType::Expansion, -1, Some(format!("{} columns", names.len()))).with_insert(text);
        item.replace_from = Some(star_from);
        items.push(item);
    }

    fn quote_if_needed(&self, name: &str) -> String {
        if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.chars().next().is_some_and(|c| c.is_ascii_digit()) && !super::scope::is_keyword(name) {
            name.to_string()
        } else {
            quote(self.dialect, name)
        }
    }

    /// Foreign keys linking `table` (either direction) with any other table.
    fn fks_for(&self, table: &SchemaTable) -> Vec<(&SchemaForeignKey, bool)> {
        self.foreign_keys
            .iter()
            .flat_map(|fk| {
                let mut v = Vec::new();
                if fk.table.eq_ignore_ascii_case(&table.name) && schema_matches(fk.schema.as_deref(), table.schema.as_deref()) {
                    v.push((fk, true));
                }
                if fk.ref_table.eq_ignore_ascii_case(&table.name) && schema_matches(fk.ref_schema.as_deref(), table.schema.as_deref()) {
                    v.push((fk, false));
                }
                v
            })
            .collect()
    }

    fn table_text(&self, schema: Option<&str>, name: &str) -> String {
        match schema {
            Some(s) if !self.default_schema.as_deref().is_some_and(|d| d.eq_ignore_ascii_case(s)) => format!("{s}.{name}"),
            _ => name.to_string(),
        }
    }

    /// After FROM/JOIN: `JOIN Customers c ON c.Id = o.CustomerId` for every FK touching a visible table.
    fn add_fk_joins(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext, with_join_keyword: bool) {
        let Some(b) = ctx.block else { return };
        let visible: Vec<&Source> = ctx.analysis.visible_sources(b);
        let used: Vec<String> = visible.iter().filter_map(|s| s.reference()).map(|r| r.to_lowercase()).collect();
        for src in &visible {
            let SourceKind::Table(parts) = &src.kind else { continue };
            let Some(table) = self.find_table(&parts.join(".")) else { continue };
            let src_ref = src.reference().unwrap_or(&table.name).to_string();
            for (fk, outgoing) in self.fks_for(table) {
                let (other_schema, other_name, mine, theirs) = if outgoing {
                    (fk.ref_schema.as_deref(), fk.ref_table.as_str(), &fk.columns, &fk.ref_columns)
                } else {
                    (fk.schema.as_deref(), fk.table.as_str(), &fk.ref_columns, &fk.columns)
                };
                let alias = make_alias(other_name, &used);
                let cond = mine
                    .iter()
                    .zip(theirs.iter())
                    .map(|(m, t)| format!("{alias}.{} = {src_ref}.{}", self.quote_if_needed(t), self.quote_if_needed(m)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let table_text = self.table_text(other_schema, other_name);
                let body = format!("{table_text} {alias} ON {cond}");
                let insert = if with_join_keyword { format!("INNER JOIN {body}") } else { body.clone() };
                let label = if with_join_keyword { format!("JOIN {table_text} ON {}", mine.join(", ")) } else { format!("{table_text} ON {}", mine.join(", ")) };
                items.push(
                    CompletionItem::new(label, CompletionType::Join, 0, Some(format!("{} {} {}", if outgoing { "→" } else { "←" }, fk.name_or_default(), cond))).with_insert(insert),
                );
            }
        }
    }

    /// In ON: `c.Id = o.CustomerId` for FKs between the just-joined source and the others.
    fn add_fk_conditions(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext) {
        let (Some(b), Some(j)) = (ctx.block, ctx.joined_source) else { return };
        let blk = &ctx.analysis.blocks[b];
        let Some(joined) = blk.sources.get(j) else { return };
        let SourceKind::Table(jp) = &joined.kind else { return };
        let Some(jt) = self.find_table(&jp.join(".")) else { return };
        let jref = joined.reference().unwrap_or(&jt.name).to_string();
        for other in ctx.analysis.visible_sources(b) {
            if std::ptr::eq(other, joined) {
                continue;
            }
            let SourceKind::Table(op) = &other.kind else { continue };
            let Some(ot) = self.find_table(&op.join(".")) else { continue };
            let oref = other.reference().unwrap_or(&ot.name).to_string();
            for (fk, outgoing) in self.fks_for(jt) {
                let (other_name, other_schema, mine, theirs) = if outgoing {
                    (fk.ref_table.as_str(), fk.ref_schema.as_deref(), &fk.columns, &fk.ref_columns)
                } else {
                    (fk.table.as_str(), fk.schema.as_deref(), &fk.ref_columns, &fk.columns)
                };
                if !other_name.eq_ignore_ascii_case(&ot.name) || !schema_matches(other_schema, ot.schema.as_deref()) {
                    continue;
                }
                let cond = mine
                    .iter()
                    .zip(theirs.iter())
                    .map(|(m, t)| format!("{jref}.{} = {oref}.{}", self.quote_if_needed(m), self.quote_if_needed(t)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                items.push(CompletionItem::new(cond.clone(), CompletionType::Join, -1, Some(fk.name_or_default())).with_insert(cond));
            }
        }
    }

    /// `INSERT INTO T |` → `(a, b, c) VALUES (, , )`.
    fn add_insert_template(&self, items: &mut Vec<CompletionItem>, ctx: &SqlContext) {
        let Some(b) = ctx.block else { return };
        let Some(target) = &ctx.analysis.blocks[b].target else { return };
        let cols = self.source_columns(&ctx.analysis, ctx.statement, target, 0);
        if cols.is_empty() {
            return;
        }
        let names: Vec<String> = cols.iter().map(|c| self.quote_if_needed(&c.name)).collect();
        let values: Vec<String> = cols.iter().enumerate().map(|(i, c)| format!("${{{}:{}}}", i + 1, c.name)).collect();
        let text = format!("({})\nVALUES ({})$0", names.join(", "), values.join(", "));
        items.push(CompletionItem::new("(columns...) VALUES (...)", CompletionType::Snippet, -1, Some(format!("{} columns", cols.len()))).with_insert(text));
        let select_text = format!("({})\nSELECT {}\nFROM $0", names.join(", "), names.join(", "));
        items.push(CompletionItem::new("(columns...) SELECT ... FROM", CompletionType::Snippet, 0, Some("insert from a query".into())).with_insert(select_text));
    }

    // --- hover -------------------------------------------------------------------------

    pub fn hover(&self, sql: &str, caret: usize) -> Option<HoverInfo> {
        let ctx = parser::analyze_context(sql, caret + 1);
        let a = &ctx.analysis;
        let ti = a.tokens.iter().position(|t| t.start <= caret && caret < t.end)?;
        let tok = &a.tokens[ti];
        if !tok.is_name() {
            return None;
        }
        let span = (tok.start, tok.end);
        let info = |title: String, detail: String| Some(HoverInfo { title, detail, start: span.0, end: span.1 });
        let name = &tok.value;
        let block = a.statement_at(tok.start).and_then(|s| a.block_at(s, tok.start));
        let visible: Vec<&Source> = block.map(|b| a.visible_sources(b)).unwrap_or_default();

        // alias.column
        if ti >= 2 && a.tokens[ti - 1].punct(".") && a.tokens[ti - 2].is_name() {
            let q = &a.tokens[ti - 2].value;
            if let Some(src) = visible.iter().find(|s| s.reference().is_some_and(|r| r.eq_ignore_ascii_case(q))) {
                let cols = self.source_columns(a, ctx.statement, src, 0);
                if let Some(c) = cols.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
                    return info(format!("{}.{}", src.reference().unwrap_or(""), c.name), column_detail(c, &self.source_label(src, a, ctx.statement)));
                }
            }
            if let Some(t) = self.find_table(&format!("{q}.{name}")) {
                return info(table_title(t), self.table_detail(t));
            }
            if let Some(r) = self.routines.iter().find(|r| r.name.eq_ignore_ascii_case(name) && r.schema.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(q))) {
                return info(routine_signature(r), routine_detail(r));
            }
        }
        if tok.kind == super::lexer::TokenKind::Variable {
            if let Some(v) = a.variables.iter().find(|v| v.name.eq_ignore_ascii_case(name)) {
                return info(v.name.clone(), v.data_type.clone());
            }
            if let Some(t) = a.temp_table(name) {
                return info(t.name.clone(), format!("table variable: {}", t.columns.join(", ")));
            }
        }
        if let Some(t) = a.temp_table(name) {
            return info(t.name.clone(), format!("temp table: {}", t.columns.join(", ")));
        }
        // A source reference (alias or table name in scope).
        if let Some(src) = visible.iter().find(|s| s.reference().is_some_and(|r| r.eq_ignore_ascii_case(name)) || s.bare_name().is_some_and(|r| r.eq_ignore_ascii_case(name))) {
            if let SourceKind::Table(p) = &src.kind {
                if let Some(t) = self.find_table(&p.join(".")) {
                    let mut title = table_title(t);
                    if let Some(al) = &src.alias {
                        title = format!("{title} AS {al}");
                    }
                    return info(title, self.table_detail(t));
                }
            }
            let cols = self.source_columns(a, ctx.statement, src, 0);
            return info(self.source_label(src, a, ctx.statement), format!("{} columns: {}", cols.len(), cols.iter().map(|c| c.name.clone()).take(30).collect::<Vec<_>>().join(", ")));
        }
        if let Some(r) = self.routines.iter().find(|r| r.name.eq_ignore_ascii_case(name)) {
            return info(routine_signature(r), routine_detail(r));
        }
        if let Some(t) = self.find_table(name) {
            return info(table_title(t), self.table_detail(t));
        }
        // An unqualified column of a visible source.
        for src in &visible {
            let cols = self.source_columns(a, ctx.statement, src, 0);
            if let Some(c) = cols.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
                return info(format!("{}.{}", src.reference().unwrap_or(""), c.name), column_detail(c, &self.source_label(src, a, ctx.statement)));
            }
        }
        None
    }

    fn table_detail(&self, t: &SchemaTable) -> String {
        let cols = self.schema.columns(&match &t.schema {
            Some(s) => format!("{s}.{}", t.name),
            None => t.name.clone(),
        });
        let list: Vec<String> = cols.iter().take(40).map(|c| format!("{} {}{}", c.name, c.data_type, if c.is_primary_key { " PK" } else { "" })).collect();
        format!("{} with {} columns\n{}", if t.is_view { "view" } else { "table" }, cols.len(), list.join("\n"))
    }

    // --- diagnostics ----------------------------------------------------------------------

    /// Unknown `alias.` qualifiers and `alias.column` names that the source doesn't have.
    pub fn diagnostics(&self, sql: &str) -> Vec<Diagnostic> {
        let a = super::scope::analyze(sql);
        let mut out = Vec::new();
        let n = a.tokens.len();
        let mut i = 0;
        while i + 2 < n {
            let (q, dot, c) = (&a.tokens[i], &a.tokens[i + 1], &a.tokens[i + 2]);
            let three_part = i + 4 < n && a.tokens[i + 3].punct(".") && a.tokens[i + 4].is_name();
            let preceded_by_dot = i > 0 && a.tokens[i - 1].punct(".");
            if !(q.is_name() && dot.punct(".") && (c.is_name() || c.punct("*")) && !preceded_by_dot && !three_part) {
                i += 1;
                continue;
            }
            let Some(st) = a.statement_at(q.start) else { i += 1; continue };
            let Some(b) = a.block_at(st, q.start) else { i += 1; continue };
            let in_from = a.clause_at(b, q.end).is_some_and(|cl| matches!(cl.kind, super::scope::ClauseKind::From | super::scope::ClauseKind::JoinTable | super::scope::ClauseKind::InsertTarget | super::scope::ClauseKind::UpdateTarget | super::scope::ClauseKind::DeleteTarget | super::scope::ClauseKind::Using | super::scope::ClauseKind::Into));
            let prev_exec = i > 0 && a.tokens[i - 1].is_any(&["EXEC", "EXECUTE", "CALL", "FROM", "JOIN", "INTO", "UPDATE", "TABLE", "VIEW", "PROCEDURE", "FUNCTION", "ON", "INDEX"]);
            if in_from || prev_exec {
                i += 3;
                continue;
            }
            let visible = a.visible_sources(b);
            let src = visible.iter().find(|s| s.reference().is_some_and(|r| r.eq_ignore_ascii_case(&q.value)) || s.bare_name().is_some_and(|r| r.eq_ignore_ascii_case(&q.value)));
            match src {
                None => {
                    if !self.is_schema_name(&q.value) && self.find_table(&q.value).is_none() && a.temp_table(&q.value).is_none() && q.kind != super::lexer::TokenKind::Variable {
                        out.push(Diagnostic { start: q.start, end: q.end, message: format!("Unknown alias or table '{}'", q.value), severity: "warning".into() });
                    }
                }
                Some(src) => {
                    if c.is_name() {
                        let cols = self.source_columns(&a, Some(st), src, 0);
                        if !cols.is_empty() && !cols.iter().any(|x| x.name.eq_ignore_ascii_case(&c.value)) {
                            out.push(Diagnostic { start: c.start, end: c.end, message: format!("Column '{}' not found in '{}'", c.value, q.value), severity: "error".into() });
                        }
                    }
                }
            }
            i += 3;
        }
        // Unknown tables in FROM / JOIN (only when the schema has tables at all).
        if !self.tables.is_empty() {
            for blk in &a.blocks {
                for s in &blk.sources {
                    if let SourceKind::Table(p) = &s.kind {
                        if p.len() <= 2 && self.find_table(&p.join(".")).is_none() && !(p.len() == 1 && a.temp_table(&p[0]).is_some()) {
                            let (start, end) = a.span(s.start, s.end.min(s.start + p.len() * 2 - 1));
                            out.push(Diagnostic { start, end, message: format!("Unknown table '{}'", p.join(".")), severity: "warning".into() });
                        }
                    }
                }
            }
        }
        out.sort_by_key(|d| d.start);
        out
    }
}

trait FkName {
    fn name_or_default(&self) -> String;
}
impl FkName for SchemaForeignKey {
    fn name_or_default(&self) -> String {
        format!("FK {}({}) → {}({})", self.table, self.columns.join(", "), self.ref_table, self.ref_columns.join(", "))
    }
}

fn schema_matches(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        _ => true,
    }
}

/// A short alias for a table that isn't already used: initials of its words (`OrderItems` →
/// `oi`), the first letter of a single word (`Users` → `u`), numbered when taken.
fn make_alias(table: &str, used: &[String]) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in table.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                segments.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            segments.push(std::mem::take(&mut cur));
        }
        cur.push(c);
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    if !cur.is_empty() {
        segments.push(cur);
    }
    let mut initials: String = segments.iter().filter_map(|s| s.chars().next()).map(|c| c.to_ascii_lowercase()).collect();
    if initials.is_empty() {
        initials = "t".into();
    }
    let mut alias = initials.clone();
    let mut n = 1;
    while used.contains(&alias.to_lowercase()) || super::scope::is_keyword(&alias) {
        n += 1;
        alias = format!("{initials}{n}");
    }
    alias
}

fn column_item(c: &SchemaColumn, qualifier: Option<&str>, priority: i32) -> CompletionItem {
    let mut desc = c.data_type.clone();
    if c.is_primary_key {
        desc.push_str(if desc.is_empty() { "PK" } else { " (PK)" });
    }
    if c.is_nullable {
        desc.push_str(" NULL");
    }
    let text = match qualifier {
        Some(q) => format!("{q}.{}", c.name),
        None => c.name.clone(),
    };
    CompletionItem::new(text, CompletionType::Column, priority, (!desc.is_empty()).then_some(desc))
}

fn table_item(t: &SchemaTable, text: String, priority: i32) -> CompletionItem {
    let desc = match &t.schema {
        Some(s) => format!("{s}.{}", t.name),
        None => t.name.clone(),
    };
    CompletionItem::new(text, if t.is_view { CompletionType::View } else { CompletionType::Table }, priority, Some(desc))
}

fn routine_signature(r: &SchemaRoutine) -> String {
    let params: Vec<String> = r.parameters.iter().map(|(n, t)| format!("{n} {t}")).collect();
    let name = match &r.schema {
        Some(s) => format!("{s}.{}", r.name),
        None => r.name.clone(),
    };
    if r.is_function {
        format!("{name}({}){}", params.join(", "), if r.returns.is_empty() { String::new() } else { format!(" RETURNS {}", r.returns) })
    } else {
        format!("{name} {}", params.join(", "))
    }
}

fn routine_detail(r: &SchemaRoutine) -> String {
    format!("{} with {} parameter(s)", if r.is_function { "function" } else { "procedure" }, r.parameters.len())
}

fn routine_item(r: &SchemaRoutine, priority: i32) -> CompletionItem {
    let text = match &r.schema {
        Some(s) => format!("{s}.{}", r.name),
        None => r.name.clone(),
    };
    let insert = if r.is_function { format!("{text}($0)") } else { text.clone() };
    CompletionItem::new(text, if r.is_function { CompletionType::Function } else { CompletionType::Procedure }, priority, Some(routine_signature(r))).with_insert(insert)
}

fn table_title(t: &SchemaTable) -> String {
    match &t.schema {
        Some(s) => format!("{s}.{}", t.name),
        None => t.name.clone(),
    }
}

fn column_detail(c: &SchemaColumn, source: &str) -> String {
    let mut s = String::new();
    if !c.data_type.is_empty() {
        s.push_str(&c.data_type);
    }
    if c.is_primary_key {
        s.push_str(" PK");
    }
    if c.is_nullable {
        s.push_str(" NULL");
    } else if !c.data_type.is_empty() {
        s.push_str(" NOT NULL");
    }
    format!("{}\n{}", s.trim(), source)
}

/// Match quality of `text` for what was typed: prefix > camel-hump initials > word boundary >
/// substring > subsequence. `None` means not a match.
pub fn match_score(partial: &str, text: &str) -> Option<i32> {
    if partial.is_empty() {
        return Some(50);
    }
    let p = partial.to_lowercase();
    let t = text.to_lowercase();
    if t == p {
        return Some(110);
    }
    if t.starts_with(&p) {
        return Some(100);
    }
    // Initials of segments (split on `_`, `.`, space, and lower→upper boundaries).
    let mut initials = String::new();
    let mut prev_lower = false;
    let mut at_boundary = true;
    for c in text.chars() {
        if c == '_' || c == '.' || c == ' ' || c == '(' {
            at_boundary = true;
            prev_lower = false;
            continue;
        }
        if at_boundary || (c.is_uppercase() && prev_lower) {
            initials.push(c.to_ascii_lowercase());
        }
        at_boundary = false;
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    if initials.len() >= 2 && initials.starts_with(&p) {
        return Some(85);
    }
    // Word-boundary prefix: `name` matches `first_name`.
    if t.split(|c: char| c == '_' || c == '.' || c == ' ').skip(1).any(|w| w.starts_with(&p)) {
        return Some(70);
    }
    if p.chars().count() >= 2 && t.contains(&p) {
        return Some(55);
    }
    if p.chars().count() >= 3 {
        let mut it = t.chars();
        if p.chars().all(|pc| it.any(|tc| tc == pc)) {
            return Some(30);
        }
    }
    None
}

fn rank_key(item: &CompletionItem, recent: &[String]) -> (i32, i32, usize, String) {
    let recency = recent.iter().position(|r| r.eq_ignore_ascii_case(&item.text)).unwrap_or(usize::MAX);
    (-item.score, item.priority, recency, item.text.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intellisense::{SchemaColumn, SchemaTable};

    struct Mock;
    impl SchemaProvider for Mock {
        fn tables(&self) -> Vec<SchemaTable> {
            ["Users", "Orders", "Products"].iter().map(|n| SchemaTable { name: n.to_string(), schema: None, is_view: false }).collect()
        }
        fn columns(&self, t: &str) -> Vec<SchemaColumn> {
            let cols: &[(&str, &str, bool)] = match t.to_lowercase().as_str() {
                "users" => &[("Id", "int", true), ("Name", "varchar", false), ("Email", "varchar", false)],
                "orders" => &[("Id", "int", true), ("UserId", "int", false), ("Total", "decimal", false)],
                "products" => &[("Id", "int", true), ("Name", "varchar", false), ("Price", "decimal", false)],
                _ => &[],
            };
            cols.iter().map(|(n, t, pk)| SchemaColumn { name: n.to_string(), data_type: t.to_string(), is_nullable: false, is_primary_key: *pk }).collect()
        }
        fn keywords(&self) -> Vec<String> {
            ["SELECT", "FROM", "WHERE", "JOIN", "INSERT", "UPDATE", "DELETE"].iter().map(|s| s.to_string()).collect()
        }
        fn foreign_keys(&self) -> Vec<SchemaForeignKey> {
            vec![SchemaForeignKey { schema: None, table: "Orders".into(), columns: vec!["UserId".into()], ref_schema: None, ref_table: "Users".into(), ref_columns: vec!["Id".into()] }]
        }
        fn routines(&self) -> Vec<SchemaRoutine> {
            vec![
                SchemaRoutine { name: "GetUser".into(), schema: None, is_function: false, parameters: vec![("@Id".into(), "int".into())], returns: String::new() },
                SchemaRoutine { name: "fnTotal".into(), schema: None, is_function: true, parameters: vec![("@OrderId".into(), "int".into())], returns: "decimal".into() },
            ]
        }
    }

    fn provider() -> IntelliSenseProvider {
        IntelliSenseProvider::new(Box::new(Mock))
    }

    fn at_end(p: &IntelliSenseProvider, sql: &str) -> Vec<CompletionItem> {
        p.completions(sql, sql.chars().count(), &[])
    }

    fn has(items: &[CompletionItem], text: &str, kind: CompletionType) -> bool {
        items.iter().any(|i| i.text == text && i.kind == kind)
    }

    #[test]
    fn from_clause_returns_tables() {
        let items = provider().completions("SELECT * FROM ", 14, &[]);
        assert!(has(&items, "Users", CompletionType::Table));
        assert!(has(&items, "Orders", CompletionType::Table));
    }

    #[test]
    fn select_list_offers_columns_from_from_clause_after_caret() {
        let items = provider().completions("SELECT  FROM Users u", 7, &[]);
        assert!(has(&items, "Name", CompletionType::Column));
        assert!(has(&items, "u", CompletionType::Alias));
        // Columns before keywords when nothing is typed.
        assert!(items.iter().position(|i| i.text == "Name") < items.iter().position(|i| i.text == "SELECT"));
    }

    #[test]
    fn dot_notation_returns_columns_for_alias() {
        let items = provider().completions("SELECT u. FROM Users u", 9, &[]);
        for c in ["Id", "Name", "Email"] {
            assert!(items.iter().any(|i| i.text == c), "missing {c}");
        }
        assert!(!items.iter().any(|i| i.text == "Total"));
    }

    #[test]
    fn new_statement_on_a_new_line_offers_select_first() {
        let p = provider();
        let sql = "SELECT * FROM Users WHERE Id = 'x'\n\nSel";
        let items = at_end(&p, sql);
        assert_eq!(items[0].text, "SELECT");
        assert!(!items.iter().any(|i| i.kind == CompletionType::Column));
    }

    #[test]
    fn from_ranks_first_in_a_select_list() {
        let p = provider();
        let items = at_end(&p, "SELECT * f");
        assert_eq!(items[0].text, "FROM");
        let items = at_end(&p, "SELECT * fro");
        assert_eq!(items[0].text, "FROM");
    }

    #[test]
    fn literals_already_in_the_editor_are_suggested() {
        let p = provider();
        let sql = "SELECT * FROM Users WHERE Id = '0a1b2c3d-1111-4222-8333-444455556666'\n\nSELECT * FROM Orders WHERE UserId = '085";
        let items = at_end(&p, sql);
        assert_eq!(items[0].kind, CompletionType::Value);
        assert_eq!(items[0].text, "0a1b2c3d-1111-4222-8333-444455556666");
        assert_eq!(items[0].insert_text.as_deref(), Some("'0a1b2c3d-1111-4222-8333-444455556666'"));
        assert_eq!(items[0].replace_from, Some(sql.len() - 4));
        assert!(items.iter().all(|i| i.kind == CompletionType::Value));
        // After `=` without quotes the value is offered quoted, alongside columns.
        let items = at_end(&p, "SELECT * FROM Users WHERE Name = 'Ann'; SELECT * FROM Users WHERE Name = ");
        let v = items.iter().find(|i| i.kind == CompletionType::Value).unwrap();
        assert_eq!(v.insert_text.as_deref(), Some("'Ann'"));
        assert!(items.iter().any(|i| i.kind == CompletionType::Column));
        // Not offered in a select list.
        let items = at_end(&p, "SELECT * FROM Users WHERE Name = 'Ann'; SELECT ");
        assert!(!items.iter().any(|i| i.kind == CompletionType::Value));
    }

    #[test]
    fn empty_query_returns_keywords_but_no_snippets() {
        let items = provider().completions("", 0, &[]);
        assert!(items.iter().any(|i| i.kind == CompletionType::Keyword));
        assert!(!items.iter().any(|i| i.kind == CompletionType::Snippet));
        let items = provider().completions("ss", 2, &[]);
        assert!(has(&items, "ssf", CompletionType::Snippet));
    }

    #[test]
    fn where_clause_returns_columns_and_keywords() {
        let items = provider().completions("SELECT * FROM Users u WHERE ", 28, &[]);
        assert!(items.iter().any(|i| i.kind == CompletionType::Column));
        assert!(items.iter().any(|i| i.kind == CompletionType::Keyword));
    }

    #[test]
    fn insert_into_returns_tables_and_then_a_template() {
        let p = provider();
        assert!(has(&p.completions("INSERT INTO ", 12, &[]), "Users", CompletionType::Table));
        let items = at_end(&p, "INSERT INTO Users ");
        let t = items.iter().find(|i| i.kind == CompletionType::Snippet).expect("template");
        assert!(t.insert_text.as_ref().unwrap().starts_with("(Id, Name, Email)\nVALUES ("));
        let cols = at_end(&p, "INSERT INTO Users (Id, ");
        assert!(has(&cols, "Name", CompletionType::Column));
        assert!(!has(&cols, "Id", CompletionType::Column));
    }

    #[test]
    fn filter_by_prefix() {
        let items = provider().completions("SELECT * FROM Us", 16, &[]);
        assert!(items.iter().any(|i| i.text == "Users"));
        assert!(!items.iter().any(|i| i.text == "Orders"));
    }

    #[test]
    fn fuzzy_ranking() {
        assert_eq!(match_score("", "x"), Some(50));
        assert_eq!(match_score("cus", "Customers"), Some(100));
        assert_eq!(match_score("cfn", "CustomerFirstName"), Some(85));
        assert_eq!(match_score("fn", "first_name"), Some(85));
        assert_eq!(match_score("name", "first_name"), Some(70));
        assert_eq!(match_score("ome", "Customers"), Some(55));
        assert_eq!(match_score("cst", "Customers"), Some(30));
        assert_eq!(match_score("zzz", "Customers"), None);
    }

    #[test]
    fn recent_items_rank_first_among_equals() {
        let p = provider();
        let items = p.completions("SELECT  FROM Users u", 7, &["Email".into()]);
        let cols: Vec<&str> = items.iter().filter(|i| i.kind == CompletionType::Column).map(|i| i.text.as_str()).collect();
        assert_eq!(cols[0], "Email");
    }

    #[test]
    fn star_expansion() {
        let p = provider();
        let items = p.completions("SELECT * FROM Users u", 8, &[]);
        let e = items.iter().find(|i| i.kind == CompletionType::Expansion).expect("expansion");
        assert_eq!(e.insert_text.as_deref(), Some("Id, Name, Email"));
        assert_eq!(e.replace_from, Some(7));
        let items = p.completions("SELECT u.* FROM Users u JOIN Orders o ON 1=1", 10, &[]);
        let e = items.iter().find(|i| i.kind == CompletionType::Expansion).unwrap();
        assert_eq!(e.insert_text.as_deref(), Some("u.Id, u.Name, u.Email"));
    }

    #[test]
    fn fk_joins_and_conditions() {
        let p = provider();
        let items = at_end(&p, "SELECT * FROM Orders o ");
        let j = items.iter().find(|i| i.kind == CompletionType::Join).expect("join suggestion");
        assert_eq!(j.insert_text.as_deref(), Some("INNER JOIN Users u ON u.Id = o.UserId"));
        let items = at_end(&p, "SELECT * FROM Orders o LEFT JOIN ");
        let j = items.iter().find(|i| i.kind == CompletionType::Join).unwrap();
        assert_eq!(j.insert_text.as_deref(), Some("Users u ON u.Id = o.UserId"));
        let items = at_end(&p, "SELECT * FROM Orders o LEFT JOIN Users usr ON ");
        let c = items.iter().find(|i| i.kind == CompletionType::Join).unwrap();
        assert_eq!(c.text, "usr.Id = o.UserId");
        assert_eq!(items[0].kind, CompletionType::Join);
    }

    #[test]
    fn cte_derived_and_temp_columns_resolve() {
        let p = provider();
        let items = at_end(&p, "WITH recent AS (SELECT Id, Total AS Amount FROM Orders) SELECT * FROM recent r WHERE r.");
        assert!(has(&items, "Amount", CompletionType::Column));
        assert!(has(&items, "Id", CompletionType::Column));
        let items = at_end(&p, "SELECT * FROM (SELECT u.* FROM Users u) d WHERE d.");
        assert!(has(&items, "Email", CompletionType::Column));
        let items = at_end(&p, "CREATE TABLE #t (A INT, B INT); SELECT * FROM #t WHERE #t.");
        assert!(has(&items, "A", CompletionType::Column));
        let items = at_end(&p, "DECLARE @x INT; SELECT * FROM Users WHERE Id = @");
        assert!(has(&items, "@x", CompletionType::Variable));
        let items = at_end(&p, "SELECT * FROM Orders o WHERE o.UserId IN (SELECT Id FROM Users u WHERE u.Name = o.");
        assert!(has(&items, "Total", CompletionType::Column));
    }

    #[test]
    fn routines_and_exec() {
        let p = provider();
        let items = at_end(&p, "EXEC ");
        assert!(has(&items, "GetUser", CompletionType::Procedure));
        let items = at_end(&p, "SELECT fn");
        let f = items.iter().find(|i| i.kind == CompletionType::Function).unwrap();
        assert_eq!(f.insert_text.as_deref(), Some("fnTotal($0)"));
    }

    #[test]
    fn ambiguous_columns_get_qualified_variants() {
        let items = at_end(&provider(), "SELECT * FROM Users u JOIN Orders o ON o.UserId = u.Id WHERE ");
        assert!(has(&items, "u.Id", CompletionType::Column));
        assert!(has(&items, "o.Id", CompletionType::Column));
        assert!(has(&items, "Total", CompletionType::Column));
        assert!(!has(&items, "o.Total", CompletionType::Column));
    }

    #[test]
    fn hover_info() {
        let p = provider();
        let sql = "SELECT u.Email FROM Users u";
        let h = p.hover(sql, 10).unwrap();
        assert_eq!(h.title, "u.Email");
        assert!(h.detail.starts_with("varchar NOT NULL"));
        let h = p.hover(sql, 21).unwrap();
        assert!(h.title.starts_with("Users AS u"));
        assert!(h.detail.contains("3 columns"));
        let h = p.hover("DECLARE @n INT; SELECT @n", 23).unwrap();
        assert_eq!(h.detail, "INT");
        assert!(p.hover("SELECT 1", 3).is_none());
    }

    #[test]
    fn diagnostics_flag_unknown_aliases_columns_and_tables() {
        let p = provider();
        let d = p.diagnostics("SELECT x.Id, u.Nope, u.Name FROM Users u JOIN Missing m ON m.Id = u.Id");
        assert!(d.iter().any(|x| x.message.contains("Unknown alias or table 'x'")));
        assert!(d.iter().any(|x| x.message.contains("Column 'Nope' not found")));
        assert!(d.iter().any(|x| x.message.contains("Unknown table 'Missing'")));
        assert!(!d.iter().any(|x| x.message.contains("'Name'")));
        assert!(p.diagnostics("SELECT u.Id FROM Users u WHERE u.Id IN (SELECT o.UserId FROM Orders o)").is_empty());
        assert!(p.diagnostics("EXEC dbo.GetUser 1").is_empty());
    }

    struct SchemaAware;
    impl SchemaProvider for SchemaAware {
        fn tables(&self) -> Vec<SchemaTable> {
            vec![
                SchemaTable { name: "Users".into(), schema: Some("dbo".into()), is_view: false },
                SchemaTable { name: "Orders".into(), schema: Some("dbo".into()), is_view: false },
                SchemaTable { name: "PartnerUser".into(), schema: Some("partner".into()), is_view: false },
            ]
        }
        fn columns(&self, t: &str) -> Vec<SchemaColumn> {
            let names: &[&str] = match t.to_lowercase().as_str() {
                "users" | "dbo.users" => &["Id", "Name"],
                "partneruser" | "partner.partneruser" => &["Id", "PartnerId"],
                _ => &[],
            };
            names.iter().map(|n| SchemaColumn { name: n.to_string(), data_type: "int".into(), ..Default::default() }).collect()
        }
        fn keywords(&self) -> Vec<String> {
            vec!["SELECT".into(), "FROM".into(), "WHERE".into()]
        }
        fn default_schema(&self) -> Option<String> {
            Some("dbo".into())
        }
    }

    fn schema_provider() -> IntelliSenseProvider {
        IntelliSenseProvider::new(Box::new(SchemaAware))
    }

    #[test]
    fn schema_names_and_qualification() {
        let p = schema_provider();
        let items = p.completions("SELECT * FROM ", 14, &[]);
        assert!(has(&items, "partner", CompletionType::Schema));
        assert!(has(&items, "dbo", CompletionType::Schema));
        assert!(has(&items, "partner.PartnerUser", CompletionType::Table));
        assert!(!items.iter().any(|i| i.text == "PartnerUser"));
        assert!(has(&items, "Users", CompletionType::Table));
        assert!(!items.iter().any(|i| i.text == "dbo.Users"));
        let items = at_end(&p, "SELECT DISTINCT * FROM partner");
        assert!(has(&items, "partner", CompletionType::Schema));
        let items = at_end(&p, "SELECT * FROM partner.");
        assert!(has(&items, "PartnerUser", CompletionType::Table));
        let items = at_end(&p, "SELECT * FROM partner.PartnerUser pu WHERE pu.");
        assert!(has(&items, "PartnerId", CompletionType::Column));
    }

    #[test]
    fn static_schema_resolves_qualified_and_bare_names() {
        let db = DbDatabase {
            name: "db".into(),
            loaded: true,
            tables: vec![crate::models::DbTable {
                name: "People".into(),
                schema: "dbo".into(),
                columns: vec![crate::models::DbColumn { name: "Id".into(), data_type: "int".into(), is_primary_key: true, is_nullable: false }],
            }],
            ..Default::default()
        };
        let s = StaticSchema::from_database(&db, vec![], Some("dbo".into()), SqlDialect::SqlServer);
        assert_eq!(s.columns("People").len(), 1);
        assert_eq!(s.columns("dbo.People").len(), 1);
        assert_eq!(s.columns("mydb.dbo.People").len(), 1);
        assert_eq!(s.columns("other.People").len(), 1);
        assert_eq!(s.columns("Nope").len(), 0);
    }

    #[test]
    fn aliases_are_short_and_unique() {
        assert_eq!(make_alias("Customers", &[]), "c");
        assert_eq!(make_alias("OrderItems", &[]), "oi");
        assert_eq!(make_alias("order_items", &[]), "oi");
        assert_eq!(make_alias("Users", &["u".into()]), "u2");
        assert_eq!(make_alias("Orders", &[]), "o");
    }
}
