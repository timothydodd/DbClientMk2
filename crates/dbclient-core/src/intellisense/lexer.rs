//! SQL tokenizer. Positions are char offsets into the source (the editor sends char-based
//! carets). Comments are dropped; an unterminated string or comment runs to the end of the text
//! so a half-typed literal never derails the rest of the analysis.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Bare identifier or keyword (`Users`, `SELECT`).
    Word,
    /// `[x]`, `"x"`, `` `x` ``.
    QuotedIdent,
    Number,
    String,
    /// `@name` (variables, parameters) and `@@rowcount`.
    Variable,
    /// `#tmp`, `##global`.
    TempName,
    /// Any punctuation / operator, possibly multi-char (`<>`, `>=`, `::`).
    Punct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Source text as written (quotes included).
    pub text: String,
    /// Identifier without its quotes; same as `text` otherwise.
    pub value: String,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn is(&self, word: &str) -> bool {
        self.kind == TokenKind::Word && self.value.eq_ignore_ascii_case(word)
    }
    pub fn is_any(&self, words: &[&str]) -> bool {
        self.kind == TokenKind::Word && words.iter().any(|w| self.value.eq_ignore_ascii_case(w))
    }
    pub fn punct(&self, p: &str) -> bool {
        self.kind == TokenKind::Punct && self.text == p
    }
    pub fn upper(&self) -> String {
        self.value.to_uppercase()
    }
    /// A name-like token: bare word, quoted identifier, variable or temp table.
    pub fn is_name(&self) -> bool {
        matches!(self.kind, TokenKind::Word | TokenKind::QuotedIdent | TokenKind::Variable | TokenKind::TempName)
    }
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

pub fn tokenize(sql: &str) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    let push = |out: &mut Vec<Token>, kind: TokenKind, chars: &[char], s: usize, e: usize, value: Option<String>| {
        let text: String = chars[s..e].iter().collect();
        let value = value.unwrap_or_else(|| text.clone());
        out.push(Token { kind, text, value, start: s, end: e });
    };
    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Comments
        if c == '-' && i + 1 < n && chars[i + 1] == '-' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            while i < n && !(chars[i] == '*' && i + 1 < n && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        // Strings ('' escapes; N'...' prefix)
        if c == '\'' || (c == 'N' && i + 1 < n && chars[i + 1] == '\'') {
            let s = i;
            if c == 'N' {
                i += 1;
            }
            i += 1;
            while i < n {
                if chars[i] == '\'' {
                    if i + 1 < n && chars[i + 1] == '\'' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            push(&mut out, TokenKind::String, &chars, s, i, None);
            continue;
        }
        // Quoted identifiers
        if c == '[' || c == '"' || c == '`' {
            let close = if c == '[' { ']' } else { c };
            let s = i;
            i += 1;
            let vs = i;
            while i < n && chars[i] != close {
                i += 1;
            }
            let value: String = chars[vs..i].iter().collect();
            if i < n {
                i += 1;
            }
            push(&mut out, TokenKind::QuotedIdent, &chars, s, i, Some(value));
            continue;
        }
        // Variables / temp names
        if c == '@' || c == '#' {
            let s = i;
            while i < n && (chars[i] == '@' || chars[i] == '#') {
                i += 1;
            }
            while i < n && is_word_char(chars[i]) {
                i += 1;
            }
            push(&mut out, if c == '@' { TokenKind::Variable } else { TokenKind::TempName }, &chars, s, i, None);
            continue;
        }
        // Numbers
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            let s = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
            push(&mut out, TokenKind::Number, &chars, s, i, None);
            continue;
        }
        // Words
        if is_word_start(c) {
            let s = i;
            while i < n && is_word_char(chars[i]) {
                i += 1;
            }
            push(&mut out, TokenKind::Word, &chars, s, i, None);
            continue;
        }
        // Punctuation (multi-char operators first)
        let two: String = chars[i..(i + 2).min(n)].iter().collect();
        if ["<>", "!=", ">=", "<=", "::", "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "->"].contains(&two.as_str()) {
            push(&mut out, TokenKind::Punct, &chars, i, i + 2, None);
            i += 2;
            continue;
        }
        push(&mut out, TokenKind::Punct, &chars, i, i + 1, None);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sql: &str) -> Vec<(TokenKind, String)> {
        tokenize(sql).into_iter().map(|t| (t.kind, t.value)).collect()
    }

    #[test]
    fn words_quotes_and_punct() {
        let t = kinds("SELECT [Order Id], `x`, \"y\" FROM dbo.T t WHERE a <> 1.5 AND b = 'it''s' -- c\n/* d */ OR @v = #tmp");
        assert!(t.contains(&(TokenKind::QuotedIdent, "Order Id".into())));
        assert!(t.contains(&(TokenKind::Punct, "<>".into())));
        assert!(t.contains(&(TokenKind::Number, "1.5".into())));
        assert!(t.contains(&(TokenKind::String, "'it''s'".into())));
        assert!(t.contains(&(TokenKind::Variable, "@v".into())));
        assert!(t.contains(&(TokenKind::TempName, "#tmp".into())));
        assert!(!t.iter().any(|(_, v)| v.contains("c") && v.starts_with("--")));
    }

    #[test]
    fn unterminated_string_runs_to_end() {
        let t = tokenize("SELECT 'abc");
        assert_eq!(t.len(), 2);
        assert_eq!(t[1].kind, TokenKind::String);
    }

    #[test]
    fn positions_are_char_offsets() {
        let t = tokenize("SELECT 'é' , x");
        assert_eq!(t[2].text, ",");
        assert_eq!(t[2].start, 11);
        assert_eq!(t[3].start, 13);
    }
}
