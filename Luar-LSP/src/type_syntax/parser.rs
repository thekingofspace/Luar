use super::ast::{NameSeg, Param, TableField, TableTypeExpr, TypeAlias, TypeExpr};
use super::lexer::{Tok, TypeSyntaxError, lex, lex_tolerant};

const MAX_DEPTH: usize = 256;

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    depth: usize,
}

pub fn parse_type(src: &str) -> Result<TypeExpr, TypeSyntaxError> {
    let mut p = Parser::new(src)?;
    let ty = p.parse_type()?;
    p.expect_eof()?;
    Ok(ty)
}

pub fn parse_type_prefix(src: &str) -> Result<(TypeExpr, usize), TypeSyntaxError> {
    let mut p = Parser {
        toks: lex_tolerant(src),
        pos: 0,
        depth: 0,
    };
    let ty = p.parse_type()?;
    Ok((ty, p.offset()))
}

pub fn parse_alias(src: &str) -> Result<TypeAlias, TypeSyntaxError> {
    let mut p = Parser::new(src)?;
    let alias = p.parse_alias()?;
    p.expect_eof()?;
    Ok(alias)
}

pub fn parse_aliases(src: &str) -> Result<Vec<TypeAlias>, TypeSyntaxError> {
    let mut p = Parser::new(src)?;
    let mut aliases = Vec::new();
    while !matches!(p.peek(), Tok::Eof) {
        aliases.push(p.parse_alias()?);
    }
    Ok(aliases)
}

impl Parser {
    fn new(src: &str) -> Result<Parser, TypeSyntaxError> {
        Ok(Parser {
            toks: lex(src)?,
            pos: 0,
            depth: 0,
        })
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].0
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].0
    }

    fn offset(&self) -> usize {
        self.toks[self.pos.min(self.toks.len() - 1)].1
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos.min(self.toks.len() - 1)].0.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok) -> Result<(), TypeSyntaxError> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(self.err(format!("expected '{t}', found '{}'", self.peek())))
        }
    }

    fn expect_eof(&mut self) -> Result<(), TypeSyntaxError> {
        if matches!(self.peek(), Tok::Eof) {
            Ok(())
        } else {
            Err(self.err(format!("unexpected '{}' after type", self.peek())))
        }
    }

    fn expect_ident(&mut self) -> Result<String, TypeSyntaxError> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err(format!("expected a name, found '{other}'"))),
        }
    }

    fn err(&self, message: String) -> TypeSyntaxError {
        TypeSyntaxError {
            message,
            offset: self.offset(),
        }
    }

    fn enter(&mut self) -> Result<(), TypeSyntaxError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(self.err("type expression is nested too deeply".to_string()))
        } else {
            Ok(())
        }
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn parse_type(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        self.enter()?;
        let lhs = self.parse_union()?;
        let result = if self.eat(&Tok::Arrow) {
            let ret = self.parse_type()?;
            TypeExpr::Function {
                params: vec![Param::Positional {
                    name: None,
                    ty: lhs,
                }],
                ret: Box::new(ret),
            }
        } else {
            lhs
        };
        self.leave();
        Ok(result)
    }

    fn parse_union(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        let first = self.parse_intersection()?;
        if !matches!(self.peek(), Tok::Pipe) {
            return Ok(first);
        }
        let mut parts = vec![first];
        while self.eat(&Tok::Pipe) {
            parts.push(self.parse_intersection()?);
        }
        Ok(TypeExpr::Union(parts))
    }

    fn parse_intersection(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        let first = self.parse_postfix()?;
        if !matches!(self.peek(), Tok::Amp) {
            return Ok(first);
        }
        let mut parts = vec![first];
        while self.eat(&Tok::Amp) {
            parts.push(self.parse_postfix()?);
        }
        Ok(TypeExpr::Intersection(parts))
    }

    fn parse_postfix(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        let mut ty = self.parse_primary()?;
        while self.eat(&Tok::Question) {
            ty = TypeExpr::Optional(Box::new(ty));
        }
        Ok(ty)
    }

    fn parse_primary(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        self.enter()?;
        let result = match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                Ok(TypeExpr::StringLit(s))
            }
            Tok::Num(n) => {
                self.bump();
                Ok(TypeExpr::NumberLit(n))
            }
            Tok::Ident(_) => self.parse_named(),
            Tok::LBrace => self.parse_table(),
            Tok::LParen => self.parse_parens(),
            other => Err(self.err(format!("expected a type, found '{other}'"))),
        };
        self.leave();
        result
    }

    fn parse_named(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        let mut segs = Vec::new();
        loop {
            let name = self.expect_ident()?;
            let args = if self.eat(&Tok::Lt) {
                let mut args = Vec::new();
                if !matches!(self.peek(), Tok::Gt) {
                    loop {
                        args.push(self.parse_type()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&Tok::Gt)?;
                Some(args)
            } else {
                None
            };
            segs.push(NameSeg { name, args });
            if !self.eat(&Tok::Dot) {
                break;
            }
        }
        Ok(TypeExpr::Named(segs))
    }

    fn parse_table(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        self.expect(&Tok::LBrace)?;
        if self.eat(&Tok::RBrace) {
            return Ok(TypeExpr::Table(TableTypeExpr::Empty));
        }
        if self.eat(&Tok::LBracket) {
            let key = self.parse_type()?;
            self.expect(&Tok::RBracket)?;
            self.expect_field_sep()?;
            let value = self.parse_type()?;
            self.eat_sep();
            self.expect(&Tok::RBrace)?;
            return Ok(TypeExpr::Table(TableTypeExpr::Indexer {
                key: Box::new(key),
                value: Box::new(value),
            }));
        }
        if self.looks_like_record_field() {
            let mut fields = Vec::new();
            loop {
                let name = self.expect_ident()?;
                let optional = self.eat(&Tok::Question);
                self.expect_field_sep()?;
                let ty = self.parse_type()?;
                fields.push(TableField { name, optional, ty });
                let had_sep = self.eat_sep();
                if matches!(self.peek(), Tok::RBrace) {
                    break;
                }
                if !had_sep {
                    return Err(
                        self.err(format!("expected ',' or '}}', found '{}'", self.peek()))
                    );
                }
            }
            self.expect(&Tok::RBrace)?;
            return Ok(TypeExpr::Table(TableTypeExpr::Record(fields)));
        }
        let elem = self.parse_type()?;
        self.eat_sep();
        self.expect(&Tok::RBrace)?;
        Ok(TypeExpr::Table(TableTypeExpr::Array(Box::new(elem))))
    }

    fn looks_like_record_field(&self) -> bool {
        if !matches!(self.peek(), Tok::Ident(_)) {
            return false;
        }
        match self.peek_at(1) {
            Tok::Colon | Tok::Equals => true,
            Tok::Question => matches!(self.peek_at(2), Tok::Colon | Tok::Equals),
            _ => false,
        }
    }

    fn expect_field_sep(&mut self) -> Result<(), TypeSyntaxError> {
        if self.eat(&Tok::Colon) || self.eat(&Tok::Equals) {
            Ok(())
        } else {
            Err(self.err(format!("expected ':' or '=', found '{}'", self.peek())))
        }
    }

    fn eat_sep(&mut self) -> bool {
        self.eat(&Tok::Comma) || self.eat(&Tok::Semi)
    }

    fn parse_parens(&mut self) -> Result<TypeExpr, TypeSyntaxError> {
        self.expect(&Tok::LParen)?;
        let mut params: Vec<Param> = Vec::new();
        if !matches!(self.peek(), Tok::RParen) {
            loop {
                if self.eat(&Tok::Ellipsis) {
                    let ty = if matches!(self.peek(), Tok::Comma | Tok::RParen) {
                        None
                    } else {
                        Some(Box::new(self.parse_type()?))
                    };
                    params.push(Param::Vararg {
                        ty: ty.map(|b| *b),
                    });
                } else if matches!(self.peek(), Tok::Ident(_))
                    && matches!(self.peek_at(1), Tok::Colon)
                {
                    let name = self.expect_ident()?;
                    self.expect(&Tok::Colon)?;
                    let ty = self.parse_type()?;
                    params.push(Param::Positional {
                        name: Some(name),
                        ty,
                    });
                } else {
                    let ty = self.parse_type()?;
                    params.push(Param::Positional { name: None, ty });
                }
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(&Tok::RParen)?;
        if self.eat(&Tok::Arrow) {
            let ret = self.parse_type()?;
            return Ok(TypeExpr::Function {
                params,
                ret: Box::new(ret),
            });
        }
        if params.len() == 1 {
            if let Param::Positional { name: None, ty } = &params[0] {
                return Ok(ty.clone());
            }
        }
        let parts = params
            .into_iter()
            .map(|p| match p {
                Param::Positional { ty, .. } => ty,
                Param::Vararg { ty } => ty.unwrap_or_else(|| TypeExpr::named("any")),
            })
            .collect();
        Ok(TypeExpr::Tuple(parts))
    }

    fn parse_alias(&mut self) -> Result<TypeAlias, TypeSyntaxError> {
        let exported = if matches!(self.peek(), Tok::Ident(w) if w == "export") {
            self.bump();
            true
        } else {
            false
        };
        match self.peek() {
            Tok::Ident(w) if w == "type" => {
                self.bump();
            }
            other => {
                return Err(self.err(format!("expected 'type', found '{other}'")));
            }
        }
        let name = self.expect_ident()?;
        let mut generics = Vec::new();
        if self.eat(&Tok::Lt) {
            if !matches!(self.peek(), Tok::Gt) {
                loop {
                    generics.push(self.expect_ident()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
            }
            self.expect(&Tok::Gt)?;
        }
        self.expect(&Tok::Equals)?;
        let ty = self.parse_type()?;
        Ok(TypeAlias {
            exported,
            name,
            generics,
            ty,
        })
    }
}
