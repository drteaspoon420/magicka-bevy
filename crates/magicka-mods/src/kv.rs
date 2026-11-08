//! A documented KeyValues (KV1-style) dialect for authored definitions.
//!
//! Supported, and nothing else:
//! - `"quoted"` strings with the escapes `\"`, `\\`, `\n`, `\t`; and bare
//!   tokens made of letters, digits and `_ . - : + / #` (no whitespace,
//!   braces or quotes);
//! - `key value` pairs and `key { ... }` blocks, nested to [`MAX_DEPTH`];
//! - `// line` and `/* block */` comments;
//! - top-level `#base "relative/path.kv"` includes, resolved by the caller
//!   through the mod filesystem (never the host filesystem);
//! - repeated keys, kept in order (callers decide whether that is an error).
//!
//! Not supported (rejected, not guessed): Source-engine `[$PLATFORM]`
//! conditionals, `#include`, unterminated strings, and stray braces.
use std::fmt;

pub const MAX_DEPTH: usize = 32;
pub const MAX_BYTES: usize = 8 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub column: u32,
}
impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum KvValue {
    Str(String),
    Block(Vec<KvNode>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct KvNode {
    pub key: String,
    pub value: KvValue,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KvError {
    pub file: String,
    pub span: Span,
    pub message: String,
}
impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.span, self.message)
    }
}
impl std::error::Error for KvError {}

/// A parsed file: `#base` includes (in order) and the root nodes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KvDocument {
    pub bases: Vec<(String, Span)>,
    pub nodes: Vec<KvNode>,
}

#[derive(Debug, PartialEq)]
enum Token {
    Text(String, bool),
    Open,
    Close,
}

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: u32,
    column: u32,
    file: &'a str,
}
impl Lexer<'_> {
    fn error(&self, message: impl Into<String>) -> KvError {
        KvError {
            file: self.file.into(),
            span: self.span(),
            message: message.into(),
        }
    }
    fn span(&self) -> Span {
        Span {
            line: self.line,
            column: self.column,
        }
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }
    fn bare(c: char) -> bool {
        c.is_alphanumeric() || "_.-:+/#".contains(c)
    }
    /// The next token and where it starts.
    fn next(&mut self) -> Result<Option<(Token, Span)>, KvError> {
        loop {
            match self.chars.peek().copied() {
                None => return Ok(None),
                Some(c) if c.is_whitespace() || c == '\u{feff}' => {
                    self.bump();
                }
                Some('/') => {
                    let start = self.span();
                    self.bump();
                    match self.chars.peek() {
                        Some('/') => {
                            while self.chars.peek().is_some_and(|c| *c != '\n') {
                                self.bump();
                            }
                        }
                        Some('*') => {
                            self.bump();
                            let mut last = '\0';
                            loop {
                                let Some(c) = self.bump() else {
                                    return Err(KvError {
                                        file: self.file.into(),
                                        span: start,
                                        message: "unterminated block comment".into(),
                                    });
                                };
                                if last == '*' && c == '/' {
                                    break;
                                }
                                last = c;
                            }
                        }
                        _ => {
                            // A bare token starting with '/'.
                            let mut text = String::from("/");
                            while self.chars.peek().is_some_and(|c| Self::bare(*c)) {
                                text.push(self.bump().unwrap());
                            }
                            return Ok(Some((Token::Text(text, false), start)));
                        }
                    }
                }
                Some('{') => {
                    let s = self.span();
                    self.bump();
                    return Ok(Some((Token::Open, s)));
                }
                Some('}') => {
                    let s = self.span();
                    self.bump();
                    return Ok(Some((Token::Close, s)));
                }
                Some('"') => {
                    let s = self.span();
                    self.bump();
                    let mut text = String::new();
                    loop {
                        match self.bump() {
                            None => {
                                return Err(KvError {
                                    file: self.file.into(),
                                    span: s,
                                    message: "unterminated string".into(),
                                })
                            }
                            Some('"') => break,
                            Some('\\') => match self.bump() {
                                Some('"') => text.push('"'),
                                Some('\\') => text.push('\\'),
                                Some('n') => text.push('\n'),
                                Some('t') => text.push('\t'),
                                other => {
                                    return Err(self.error(format!(
                                        "unsupported escape \\{}",
                                        other.map_or(String::new(), String::from)
                                    )))
                                }
                            },
                            Some(c) => text.push(c),
                        }
                    }
                    return Ok(Some((Token::Text(text, true), s)));
                }
                Some('[') => return Err(self.error("platform conditionals are not supported")),
                Some(c) if Self::bare(c) => {
                    let s = self.span();
                    let mut text = String::new();
                    while self.chars.peek().is_some_and(|c| Self::bare(*c)) {
                        text.push(self.bump().unwrap());
                    }
                    return Ok(Some((Token::Text(text, false), s)));
                }
                Some(c) => return Err(self.error(format!("unexpected character {c:?}"))),
            }
        }
    }
}

/// Parse one KV file. `file` names it in diagnostics (its logical path).
pub fn parse(file: &str, text: &str) -> Result<KvDocument, KvError> {
    if text.len() > MAX_BYTES {
        return Err(KvError {
            file: file.into(),
            span: Span { line: 1, column: 1 },
            message: format!("file exceeds {MAX_BYTES} bytes"),
        });
    }
    let mut lexer = Lexer {
        chars: text.chars().peekable(),
        line: 1,
        column: 1,
        file,
    };
    let mut doc = KvDocument::default();
    // Stack of open blocks: (key, span, children).
    let mut stack: Vec<(String, Span, Vec<KvNode>)> = Vec::new();
    let mut pending: Option<(String, Span)> = None;
    while let Some((token, span)) = lexer.next()? {
        match token {
            Token::Text(text, quoted) => match pending.take() {
                None if !quoted && text == "#base" => {
                    if !stack.is_empty() {
                        return Err(lexer.error("#base is only allowed at the top level"));
                    }
                    match lexer.next()? {
                        Some((Token::Text(path, _), s)) => doc.bases.push((path, s)),
                        _ => return Err(lexer.error("#base needs a path")),
                    }
                }
                None if !quoted && text.starts_with('#') && text.len() > 1 && stack.is_empty() => {
                    return Err(KvError {
                        file: file.into(),
                        span,
                        message: format!("unsupported directive {text}"),
                    })
                }
                None => pending = Some((text, span)),
                Some((key, key_span)) => {
                    let node = KvNode {
                        key,
                        value: KvValue::Str(text),
                        span: key_span,
                    };
                    match stack.last_mut() {
                        Some(top) => top.2.push(node),
                        None => doc.nodes.push(node),
                    }
                }
            },
            Token::Open => {
                let Some((key, key_span)) = pending.take() else {
                    return Err(KvError {
                        file: file.into(),
                        span,
                        message: "a block needs a key".into(),
                    });
                };
                if stack.len() >= MAX_DEPTH {
                    return Err(KvError {
                        file: file.into(),
                        span,
                        message: format!("blocks nest deeper than {MAX_DEPTH}"),
                    });
                }
                stack.push((key, key_span, Vec::new()));
            }
            Token::Close => {
                if let Some((key, s)) = pending.take() {
                    return Err(KvError {
                        file: file.into(),
                        span: s,
                        message: format!("key {key:?} has no value"),
                    });
                }
                let Some((key, key_span, children)) = stack.pop() else {
                    return Err(KvError {
                        file: file.into(),
                        span,
                        message: "unmatched '}'".into(),
                    });
                };
                let node = KvNode {
                    key,
                    value: KvValue::Block(children),
                    span: key_span,
                };
                match stack.last_mut() {
                    Some(top) => top.2.push(node),
                    None => doc.nodes.push(node),
                }
            }
        }
    }
    if let Some((key, s)) = pending {
        return Err(KvError {
            file: file.into(),
            span: s,
            message: format!("key {key:?} has no value"),
        });
    }
    if let Some((key, s, _)) = stack.pop() {
        return Err(KvError {
            file: file.into(),
            span: s,
            message: format!("block {key:?} is not closed"),
        });
    }
    Ok(doc)
}

impl KvNode {
    pub fn str(&self) -> Option<&str> {
        match &self.value {
            KvValue::Str(s) => Some(s),
            KvValue::Block(_) => None,
        }
    }
    pub fn children(&self) -> &[KvNode] {
        match &self.value {
            KvValue::Block(c) => c,
            KvValue::Str(_) => &[],
        }
    }
    /// The first child with `key` (keys compare case-insensitively).
    pub fn get(&self, key: &str) -> Option<&KvNode> {
        self.children()
            .iter()
            .find(|n| n.key.eq_ignore_ascii_case(key))
    }
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(KvNode::str)
    }
}

/// Serialize back to canonical KV text (stable; used for hashing).
pub fn write(nodes: &[KvNode]) -> String {
    fn quote(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    fn go(nodes: &[KvNode], depth: usize, out: &mut String) {
        for n in nodes {
            out.push_str(&"\t".repeat(depth));
            quote(&n.key, out);
            match &n.value {
                KvValue::Str(s) => {
                    out.push(' ');
                    quote(s, out);
                    out.push('\n');
                }
                KvValue::Block(c) => {
                    out.push('\n');
                    out.push_str(&"\t".repeat(depth));
                    out.push_str("{\n");
                    go(c, depth + 1, out);
                    out.push_str(&"\t".repeat(depth));
                    out.push_str("}\n");
                }
            }
        }
    }
    let mut out = String::new();
    go(nodes, 0, &mut out);
    out
}
