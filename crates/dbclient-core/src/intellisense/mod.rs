//! Context-aware SQL completion: a regex tokenizer that works out what the caret
//! is in (select list, FROM clause, after `alias.`, ...) and a provider that turns
//! that into a filtered, ranked list of completions from a schema.

pub mod lexer;
pub mod parser;
pub mod scope;
pub mod provider;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod snippets;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletionType {
    Keyword,
    Table,
    View,
    Column,
    Alias,
    Schema,
    Cte,
    Function,
    Procedure,
    Variable,
    Snippet,
    /// A generated JOIN clause or ON condition from a foreign key.
    Join,
    /// `*` expanded to the column list.
    Expansion,
    /// A literal value already used elsewhere in the editor (ids, dates, names).
    Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    /// Shown in the list and matched against what was typed.
    pub text: String,
    #[serde(rename = "type")]
    pub kind: CompletionType,
    /// Lower sorts first within equal match quality.
    pub priority: i32,
    pub description: Option<String>,
    /// Text to insert when it differs from `text` (snippets, joins, expansions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    /// Char offset to replace from, when the item replaces more than the typed word.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace_from: Option<usize>,
    /// Match quality (higher is better), set by the ranker.
    #[serde(default)]
    pub score: i32,
}

impl CompletionItem {
    pub fn new(text: impl Into<String>, kind: CompletionType, priority: i32, description: Option<String>) -> Self {
        Self { text: text.into(), kind, priority, description, insert_text: None, replace_from: None, score: 0 }
    }
    pub fn with_insert(mut self, insert: impl Into<String>) -> Self {
        self.insert_text = Some(insert.into());
        self
    }
}

/// A table (or view) the completion engine knows about.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchemaTable {
    pub name: String,
    pub schema: Option<String>,
    pub is_view: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchemaColumn {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchemaForeignKey {
    pub schema: Option<String>,
    pub table: String,
    pub columns: Vec<String>,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SchemaRoutine {
    pub name: String,
    pub schema: Option<String>,
    pub is_function: bool,
    /// `(name, type)` pairs.
    pub parameters: Vec<(String, String)>,
    pub returns: String,
}

/// Where the schema comes from. Implemented over a loaded database (see
/// `provider::StaticSchema`) and by test doubles.
pub trait SchemaProvider {
    fn tables(&self) -> Vec<SchemaTable>;
    fn columns(&self, table_name: &str) -> Vec<SchemaColumn>;
    fn keywords(&self) -> Vec<String>;
    /// Schema unqualified names resolve to (`dbo` on SQL Server); `None` when the dialect has none.
    fn default_schema(&self) -> Option<String> {
        None
    }
    fn foreign_keys(&self) -> Vec<SchemaForeignKey> {
        Vec::new()
    }
    fn routines(&self) -> Vec<SchemaRoutine> {
        Vec::new()
    }
    fn dialect(&self) -> crate::SqlDialect {
        crate::SqlDialect::SqlServer
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlContextType {
    Unknown,
    General,
    SelectList,
    FromClause,
    WhereClause,
    JoinCondition,
    InsertInto,
    InsertColumns,
    InsertValues,
    UpdateTable,
    UpdateSet,
    ColumnAfterDot,
    AfterTableName,
    AfterTableAlias,
    AfterGroupOrOrder,
}

/// Case-insensitive alias map: lower-cased key -> (alias as written, full table name).
#[derive(Debug, Clone, Default)]
pub struct Aliases(pub HashMap<String, (String, String)>);

impl Aliases {
    pub fn insert(&mut self, alias: &str, table: &str) {
        self.0.entry(alias.to_lowercase()).or_insert_with(|| (alias.to_string(), table.to_string()));
    }
    pub fn get(&self, alias: &str) -> Option<&str> {
        self.0.get(&alias.to_lowercase()).map(|(_, t)| t.as_str())
    }
    pub fn contains(&self, alias: &str) -> bool {
        self.0.contains_key(&alias.to_lowercase())
    }
    pub fn keys(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.values().map(|(a, _)| a.clone()).collect();
        v.sort();
        v
    }
    pub fn tables(&self) -> Vec<String> {
        self.0.values().map(|(_, t)| t.clone()).collect()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What the caret is in. `analysis` holds the full statement structure for the provider.
#[derive(Debug, Clone, Default)]
pub struct SqlContext {
    pub kind: Option<SqlContextType>,
    /// `alias` in `alias.|`, possibly multi-part (`dbo.Orders`).
    pub table_prefix: String,
    pub expecting_column_name: bool,
    pub expecting_table_name: bool,
    pub expecting_keyword: bool,
    pub last_keyword: String,
    /// alias (or bare name) -> table name, for every visible source that has a table name.
    pub table_aliases: Aliases,
    pub available_aliases: Vec<String>,
    pub current_query: String,
    pub all_tokens: Vec<String>,
    pub analysis: scope::Analysis,
    pub statement: Option<usize>,
    pub block: Option<usize>,
    pub clause: Option<scope::ClauseKind>,
    /// The identifier being typed (empty after a dot or space) and where it starts.
    pub partial: String,
    pub word_start: usize,
    /// Caret sits right after `*` (or `t.*`) in a select list.
    pub after_star: Option<usize>,
    /// JOIN clause just before an ON, for condition suggestions.
    pub joined_source: Option<usize>,
    /// INSERT target typed, caret after it: offer the column/VALUES template.
    pub after_insert_target: bool,
    /// Caret is inside a string literal that starts at this char offset.
    pub in_string: Option<usize>,
}

impl SqlContext {
    pub fn of(kind: SqlContextType) -> Self {
        SqlContext { kind: Some(kind), ..Default::default() }
    }
    pub fn context_type(&self) -> SqlContextType {
        self.kind.unwrap_or(SqlContextType::Unknown)
    }
}

/// Hover / signature information for the identifier under the caret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverInfo {
    pub title: String,
    pub detail: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub message: String,
    /// "error" | "warning"
    pub severity: String,
}
