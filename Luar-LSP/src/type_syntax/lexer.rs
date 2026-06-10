use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    Arrow,
    Pipe,
    Amp,
    Question,
    Lt,
    Gt,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Colon,
    Equals,
    Comma,
    Semi,
    Dot,
    Ellipsis,
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Ident(s) => write!(f, "{s}"),
            Tok::Str(s) => write!(f, "\"{s}\""),
            Tok::Num(s) => write!(f, "{s}"),
            Tok::Arrow => write!(f, "->"),
            Tok::Pipe => write!(f, "|"),
            Tok::Amp => write!(f, "&"),
            Tok::Question => write!(f, "?"),
            Tok::Lt => write!(f, "<"),
            Tok::Gt => write!(f, ">"),
            Tok::LBrace => write!(f, "{{"),
            Tok::RBrace => write!(f, "}}"),
            Tok::LBracket => write!(f, "["),
            Tok::RBracket => write!(f, "]"),
            Tok::LParen => write!(f, "("),
            Tok::RParen => write!(f, ")"),
            Tok::Colon => write!(f, ":"),
            Tok::Equals => write!(f, "="),
            Tok::Comma => write!(f, ","),
            Tok::Semi => write!(f, ";"),
            Tok::Dot => write!(f, "."),
            Tok::Ellipsis => write!(f, "..."),
            Tok::Eof => write!(f, "<eof>"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeSyntaxError {
    pub message: String,
    pub offset: usize,
}

impl fmt::Display for TypeSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at offset {})", self.message, self.offset)
    }
}

impl std::error::Error for TypeSyntaxError {}

pub fn lex(src: &str) -> Result<Vec<(Tok, usize)>, TypeSyntaxError> {
    lex_impl(src, false)
}

pub fn lex_tolerant(src: &str) -> Vec<(Tok, usize)> {
    lex_impl(src, true).unwrap_or_else(|_| vec![(Tok::Eof, 0)])
}

fn lex_impl(src: &str, tolerant: bool) -> Result<Vec<(Tok, usize)>, TypeSyntaxError> {
    let bytes = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    'outer: while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            b'-' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'>' {
                    toks.push((Tok::Arrow, i));
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                } else if i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                    let start = i;
                    i += 1;
                    i = lex_number(bytes, i);
                    toks.push((Tok::Num(src[start..i].to_string()), start));
                } else if tolerant {
                    break;
                } else {
                    return Err(TypeSyntaxError {
                        message: format!("unexpected character '{}'", c as char),
                        offset: i,
                    });
                }
            }
            b'|' => {
                toks.push((Tok::Pipe, i));
                i += 1;
            }
            b'&' => {
                toks.push((Tok::Amp, i));
                i += 1;
            }
            b'?' => {
                toks.push((Tok::Question, i));
                i += 1;
            }
            b'<' => {
                toks.push((Tok::Lt, i));
                i += 1;
            }
            b'>' => {
                toks.push((Tok::Gt, i));
                i += 1;
            }
            b'{' => {
                toks.push((Tok::LBrace, i));
                i += 1;
            }
            b'}' => {
                toks.push((Tok::RBrace, i));
                i += 1;
            }
            b'[' => {
                toks.push((Tok::LBracket, i));
                i += 1;
            }
            b']' => {
                toks.push((Tok::RBracket, i));
                i += 1;
            }
            b'(' => {
                toks.push((Tok::LParen, i));
                i += 1;
            }
            b')' => {
                toks.push((Tok::RParen, i));
                i += 1;
            }
            b':' => {
                toks.push((Tok::Colon, i));
                i += 1;
            }
            b'=' => {
                toks.push((Tok::Equals, i));
                i += 1;
            }
            b',' => {
                toks.push((Tok::Comma, i));
                i += 1;
            }
            b';' => {
                toks.push((Tok::Semi, i));
                i += 1;
            }
            b'.' => {
                if i + 2 < bytes.len() && bytes[i + 1] == b'.' && bytes[i + 2] == b'.' {
                    toks.push((Tok::Ellipsis, i));
                    i += 3;
                } else {
                    toks.push((Tok::Dot, i));
                    i += 1;
                }
            }
            b'"' | b'\'' => {
                let quote = c;
                let start = i;
                i += 1;
                let mut s = String::new();
                loop {
                    if i >= bytes.len() {
                        if tolerant {
                            break 'outer;
                        }
                        return Err(TypeSyntaxError {
                            message: "unterminated string literal".to_string(),
                            offset: start,
                        });
                    }
                    let ch = bytes[i];
                    if ch == quote {
                        i += 1;
                        break;
                    }
                    if ch == b'\\' && i + 1 < bytes.len() {
                        let esc = bytes[i + 1];
                        let translated = match esc {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'\\' => '\\',
                            b'"' => '"',
                            b'\'' => '\'',
                            other => other as char,
                        };
                        s.push(translated);
                        i += 2;
                    } else {
                        s.push(ch as char);
                        i += 1;
                    }
                }
                toks.push((Tok::Str(s), start));
            }
            b'0'..=b'9' => {
                let start = i;
                i = lex_number(bytes, i);
                toks.push((Tok::Num(src[start..i].to_string()), start));
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                toks.push((Tok::Ident(src[start..i].to_string()), start));
            }
            _ => {
                if tolerant {
                    break 'outer;
                }
                return Err(TypeSyntaxError {
                    message: format!("unexpected character '{}'", c as char),
                    offset: i,
                });
            }
        }
    }
    let end = if tolerant { i } else { src.len() };
    toks.push((Tok::Eof, end.min(src.len())));
    Ok(toks)
}

fn lex_number(bytes: &[u8], mut i: usize) -> usize {
    if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
        i += 2;
        while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
            i += 1;
        }
        return i;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        if j < bytes.len() && bytes[j].is_ascii_digit() {
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}
