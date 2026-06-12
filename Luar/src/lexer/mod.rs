
mod token;

pub use token::{Span, Token, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lex error at {}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for LexError {}

const DELIMITERS: &[char] = &['(', ')', '{', '}', '[', ']', ';', ','];

const SYMBOLS: &[char] = &[
    '+', '-', '*', '/', '%', '^', '#', '&', '~', '|', '<', '>', '=', ':', '.', '?', '!', '@', '$',
];

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,

    keep_comments: bool,
}

impl Lexer {

    pub fn new(source: &str) -> Self {
        Lexer {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            keep_comments: false,
        }
    }

    pub fn keep_comments(mut self, keep: bool) -> Self {
        self.keep_comments = keep;
        self
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span(&self, start: usize, line: u32, col: u32) -> Span {
        Span { start, end: self.pos, line, col }
    }

    fn error(&self, message: impl Into<String>) -> LexError {
        LexError { message: message.into(), line: self.line, col: self.col }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia();
            let start = self.pos;
            let line = self.line;
            let col = self.col;

            let Some(c) = self.peek() else {
                tokens.push(Token::new(TokenKind::Eof, "", self.span(start, line, col)));
                break;
            };

            let token = if is_ident_start(c) {
                self.lex_identifier(start, line, col)
            } else if c.is_ascii_digit() {
                self.lex_number(start, line, col)?
            } else if c == '"' || c == '\'' {
                self.lex_string(start, line, col)?
            } else if c == '`' {
                self.lex_interp_string(start, line, col)?
            } else if c == '[' && self.long_bracket_level().is_some() {
                self.lex_long_string(start, line, col)?
            } else if c == '-' && self.peek_at(1) == Some('-') {

                let text = self.consume_comment();
                Token::new(TokenKind::Comment, text, self.span(start, line, col))
            } else if DELIMITERS.contains(&c) {
                self.bump();
                Token::new(TokenKind::Delimiter, c.to_string(), self.span(start, line, col))
            } else if SYMBOLS.contains(&c) {
                self.lex_operator(start, line, col)
            } else {
                return Err(self.error(format!("unexpected character {c:?}")));
            };

            tokens.push(token);
        }
        Ok(tokens)
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('-') if !self.keep_comments && self.peek_at(1) == Some('-') => {
                    self.consume_comment();
                }
                _ => break,
            }
        }
    }

    fn consume_comment(&mut self) -> String {
        let mut text = String::new();
        self.bump();
        self.bump();
        if self.peek() == Some('[') && self.peek_at(1) == Some('[') {

            self.bump();
            self.bump();
            while let Some(c) = self.peek() {
                if c == ']' && self.peek_at(1) == Some(']') {
                    self.bump();
                    self.bump();
                    break;
                }
                text.push(c);
                self.bump();
            }
        } else {

            while let Some(c) = self.peek() {
                if c == '\n' {
                    break;
                }
                text.push(c);
                self.bump();
            }
        }
        text
    }

    fn lex_identifier(&mut self, start: usize, line: u32, col: u32) -> Token {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                text.push(c);
                self.bump();
            } else {
                break;
            }
        }
        Token::new(TokenKind::Identifier, text, self.span(start, line, col))
    }

    fn lex_number(&mut self, start: usize, line: u32, col: u32) -> Result<Token, LexError> {
        let mut text = String::new();
        let mut is_float = false;

        if self.peek() == Some('0') && matches!(self.peek_at(1), Some('x') | Some('X')) {
            text.push(self.bump().unwrap());
            text.push(self.bump().unwrap());
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() || c == '_' {
                    text.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
            return Ok(Token::new(TokenKind::Int, text, self.span(start, line, col)));
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                text.push(c);
                self.bump();
            } else {
                break;
            }
        }

        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            text.push(self.bump().unwrap());
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '_' {
                    text.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
        }

        if matches!(self.peek(), Some('e') | Some('E')) {
            is_float = true;
            text.push(self.bump().unwrap());
            if matches!(self.peek(), Some('+') | Some('-')) {
                text.push(self.bump().unwrap());
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.error("malformed exponent in number literal"));
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    text.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
        }

        let kind = if is_float { TokenKind::Float } else { TokenKind::Int };
        Ok(Token::new(kind, text, self.span(start, line, col)))
    }

    fn lex_string(&mut self, start: usize, line: u32, col: u32) -> Result<Token, LexError> {
        let quote = self.bump().unwrap();
        let mut raw = String::new();
        loop {
            match self.bump() {
                None => return Err(self.error("unterminated string literal")),
                Some(c) if c == quote => break,
                Some('\\') => {
                    raw.push('\\');
                    let esc = self.bump().ok_or_else(|| self.error("unterminated escape"))?;
                    raw.push(esc);
                }
                Some(c) => raw.push(c),
            }
        }
        Ok(Token::new(TokenKind::Str, unescape(&raw), self.span(start, line, col)))
    }

    fn lex_long_string(&mut self, start: usize, line: u32, col: u32) -> Result<Token, LexError> {
        let level = self.long_bracket_level().unwrap();
        self.bump();
        for _ in 0..level {
            self.bump();
        }
        self.bump();
        if self.peek() == Some('\n') {
            self.bump();
        } else if self.peek() == Some('\r') && self.peek_at(1) == Some('\n') {
            self.bump();
            self.bump();
        }
        let mut value = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated long string")),
                Some(']') if self.is_long_close(level) => {
                    self.bump();
                    for _ in 0..level {
                        self.bump();
                    }
                    self.bump();
                    break;
                }
                Some(c) => {
                    value.push(c);
                    self.bump();
                }
            }
        }
        Ok(Token::new(TokenKind::Str, value, self.span(start, line, col)))
    }

    fn lex_interp_string(&mut self, start: usize, line: u32, col: u32) -> Result<Token, LexError> {
        self.bump();
        let mut raw = String::new();
        loop {
            match self.bump() {
                None => return Err(self.error("unterminated interpolated string")),
                Some('`') => break,
                Some('\\') => {
                    raw.push('\\');
                    if let Some(n) = self.bump() {
                        raw.push(n);
                    }
                }
                Some(c) => raw.push(c),
            }
        }
        Ok(Token::new(TokenKind::InterpStr, raw, self.span(start, line, col)))
    }

    fn long_bracket_level(&self) -> Option<usize> {
        if self.peek() != Some('[') {
            return None;
        }
        let mut k = 1;
        let mut eq = 0;
        while self.peek_at(k) == Some('=') {
            eq += 1;
            k += 1;
        }
        if self.peek_at(k) == Some('[') {
            Some(eq)
        } else {
            None
        }
    }

    fn is_long_close(&self, level: usize) -> bool {
        if self.peek() != Some(']') {
            return false;
        }
        let mut k = 1;
        for _ in 0..level {
            if self.peek_at(k) != Some('=') {
                return false;
            }
            k += 1;
        }
        self.peek_at(k) == Some(']')
    }

    fn lex_operator(&mut self, start: usize, line: u32, col: u32) -> Token {
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if SYMBOLS.contains(&c) {
                if c == '.' && !text.is_empty() && !text.ends_with('.') {
                    break;
                }
                if c == '<' && !text.is_empty() {
                    break;
                }
                text.push(c);
                self.bump();
            } else {
                break;
            }
        }
        Token::new(TokenKind::Operator, text, self.span(start, line, col))
    }
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(source).tokenize()
}

pub(crate) fn unescape(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                'a' => out.push('\u{07}'),
                'b' => out.push('\u{08}'),
                'f' => out.push('\u{0C}'),
                'v' => out.push('\u{0B}'),
                'e' => out.push('\u{1B}'),
                '\\' => out.push('\\'),
                '\'' => out.push('\''),
                '"' => out.push('"'),
                '`' => out.push('`'),
                '{' => out.push('{'),
                '}' => out.push('}'),
                '\n' => {}
                'x' => {
                    let mut hex = String::new();
                    while hex.len() < 2 && i + 1 < chars.len() && chars[i + 1].is_ascii_hexdigit() {
                        i += 1;
                        hex.push(chars[i]);
                    }
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                'u' => {
                    if i + 1 < chars.len() && chars[i + 1] == '{' {
                        i += 1;
                        let mut hex = String::new();
                        while i + 1 < chars.len() && chars[i + 1] != '}' {
                            i += 1;
                            hex.push(chars[i]);
                        }
                        if i + 1 < chars.len() && chars[i + 1] == '}' {
                            i += 1;
                        }
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            out.push(ch);
                        }
                    }
                }
                other => out.push(other),
            }
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_identifiers_and_numbers() {
        let toks = tokenize("foo 42 3.14").unwrap();
        assert_eq!(toks[0].kind, TokenKind::Identifier);
        assert_eq!(toks[0].text, "foo");
        assert_eq!(toks[1].kind, TokenKind::Int);
        assert_eq!(toks[2].kind, TokenKind::Float);
        assert_eq!(toks[3].kind, TokenKind::Eof);
    }

    #[test]
    fn lexes_strings_with_escapes() {
        let toks = tokenize(r#""a\nb" 'c'"#).unwrap();
        assert_eq!(toks[0].kind, TokenKind::Str);
        assert_eq!(toks[0].text, "a\nb");
        assert_eq!(toks[1].text, "c");
    }

    #[test]
    fn maximal_munch_operators() {
        let toks = tokenize("a == b .. c").unwrap();
        assert_eq!(toks[1].kind, TokenKind::Operator);
        assert_eq!(toks[1].text, "==");
        assert_eq!(toks[3].text, "..");
    }

    #[test]
    fn dots_split_from_preceding_operators() {
        let toks = tokenize(":...").unwrap();
        assert_eq!(toks[0].text, ":");
        assert_eq!(toks[1].text, "...");
        let toks = tokenize("->...").unwrap();
        assert_eq!(toks[0].text, "->");
        assert_eq!(toks[1].text, "...");
        let toks = tokenize("...:").unwrap();
        assert_eq!(toks[0].text, "...:");
        let toks = tokenize("a..b").unwrap();
        assert_eq!(toks[1].text, "..");
    }

    #[test]
    fn angle_opens_split_from_preceding_operators() {
        let toks = tokenize(":<").unwrap();
        assert_eq!(toks[0].text, ":");
        assert_eq!(toks[1].text, "<");
        let toks = tokenize("=<").unwrap();
        assert_eq!(toks[0].text, "=");
        assert_eq!(toks[1].text, "<");
        let toks = tokenize("a <= b").unwrap();
        assert_eq!(toks[1].text, "<=");
        let toks = tokenize("a < b").unwrap();
        assert_eq!(toks[1].text, "<");
    }

    #[test]
    fn skips_comments() {
        let k = kinds("a -- line\n b --[[ block ]] c");
        assert_eq!(k, vec![TokenKind::Identifier, TokenKind::Identifier, TokenKind::Identifier, TokenKind::Eof]);
    }

    #[test]
    fn subtraction_is_not_a_comment() {
        let toks = tokenize("a - b").unwrap();
        assert_eq!(toks[1].kind, TokenKind::Operator);
        assert_eq!(toks[1].text, "-");
    }

    #[test]
    fn hex_and_delimiters() {
        let toks = tokenize("0xFF (x)").unwrap();
        assert_eq!(toks[0].kind, TokenKind::Int);
        assert_eq!(toks[0].text, "0xFF");
        assert_eq!(toks[1].kind, TokenKind::Delimiter);
        assert_eq!(toks[1].text, "(");
    }
}
