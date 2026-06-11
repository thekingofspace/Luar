
use crate::ast::*;
use crate::lexer::{Token, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parse error at {}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParseError {}

type PResult<T> = Result<T, ParseError>;

pub fn parse(tokens: Vec<Token>) -> PResult<Vec<Stmt>> {
    Parser::new(tokens).parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {

        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn peek2(&self) -> &Token {
        &self.tokens[(self.pos + 1).min(self.tokens.len() - 1)]
    }

    fn peek_at(&self, n: usize) -> &Token {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)]
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    fn is_kw(&self, kw: &str) -> bool {
        let t = self.peek();
        t.kind == TokenKind::Identifier && t.text == kw
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.is_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn is_op(&self, s: &str) -> bool {
        let t = self.peek();
        t.kind == TokenKind::Operator && t.text == s
    }

    fn eat_op(&mut self, s: &str) -> bool {
        if self.is_op(s) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn is_delim(&self, c: &str) -> bool {
        let t = self.peek();
        t.kind == TokenKind::Delimiter && t.text == c
    }

    fn eat_delim(&mut self, c: &str) -> bool {
        if self.is_delim(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        let t = self.peek();
        ParseError { message: message.into(), line: t.span.line, col: t.span.col }
    }

    fn expect_op(&mut self, s: &str) -> PResult<()> {
        if self.eat_op(s) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{s}', found {:?}", self.peek().text)))
        }
    }

    fn expect_delim(&mut self, c: &str) -> PResult<()> {
        if self.eat_delim(c) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{c}', found {:?}", self.peek().text)))
        }
    }

    fn expect_kw(&mut self, kw: &str) -> PResult<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{kw}', found {:?}", self.peek().text)))
        }
    }

    fn expect_ident(&mut self) -> PResult<String> {
        if self.peek().kind == TokenKind::Identifier {
            Ok(self.bump().text)
        } else {
            Err(self.error(format!("expected identifier, found {:?}", self.peek().text)))
        }
    }

    fn parse_program(&mut self) -> PResult<Vec<Stmt>> {
        let mut stmts = Vec::new();
        loop {
            while self.eat_delim(";") {}
            if self.at_eof() {
                break;
            }
            stmts.push(self.parse_statement()?);
        }
        Ok(stmts)
    }

    fn parse_block(&mut self, terminators: &[&str]) -> PResult<Vec<Stmt>> {
        let mut stmts = Vec::new();
        loop {
            while self.eat_delim(";") {}
            if self.at_eof() {
                break;
            }
            let t = self.peek();
            if t.kind == TokenKind::Identifier && terminators.contains(&t.text.as_str()) {
                break;
            }
            stmts.push(self.parse_statement()?);
        }
        Ok(stmts)
    }

    fn parse_statement(&mut self) -> PResult<Stmt> {

        let exported = self.eat_kw("export");
        if self.is_kw("return") {
            let line = self.peek().span.line;
            self.bump();
            let values = if self.at_block_end() { Vec::new() } else { self.parse_expr_list()? };
            return Ok(Stmt::Return { values, line });
        }

        if self.is_kw("type") && self.peek2().kind == TokenKind::Identifier {
            return self.parse_type_alias();
        }

        if self.modifier_then("class") {
            return self.parse_class();
        }
        if self.modifier_then("interface") {
            return self.parse_interface();
        }
        if self.modifier_then("enum") {
            return self.parse_enum();
        }

        if self.is_kw("buff")
            && matches!(self.peek2().kind, TokenKind::Int | TokenKind::Float)
        {
            return self.parse_buff();
        }

        if self.is_kw("freebuff") && self.peek2().kind == TokenKind::Identifier {
            let line = self.peek().span.line;
            self.bump();
            let name = self.expect_ident()?;
            return Ok(Stmt::FreeBuff { name, line });
        }
        if self.is_kw("pub") && self.peek2().kind == TokenKind::Identifier && self.peek2().text == "buff" {
            return Err(self.error("a buff cannot be `pub` (buffs are always local)"));
        }

        if self.is_kw("pub") || self.is_kw("local") || self.is_kw("const") || self.is_kw("function") {
            return self.parse_declaration_or_function(exported);
        }
        if self.is_kw("do") {
            self.bump();
            let body = self.parse_block(&["end"])?;
            self.expect_kw("end")?;
            return Ok(Stmt::Do(body));
        }
        if self.is_kw("if") {
            return self.parse_if();
        }
        if self.is_kw("while") {
            return self.parse_while();
        }
        if self.is_kw("for") {
            return self.parse_for();
        }
        if self.is_kw("break") {
            let line = self.peek().span.line;
            self.bump();
            return Ok(Stmt::Break { line });
        }
        self.parse_expr_or_assign()
    }

    fn parse_declaration_or_function(&mut self, exported: bool) -> PResult<Stmt> {
        let line = self.peek().span.line;
        let visibility = if exported || self.eat_kw("pub") { Visibility::Pub } else { Visibility::Local };

        let mutability = if self.eat_kw("const") {
            Mutability::Const
        } else if self.eat_kw("local") {
            Mutability::Mutable
        } else {
            Mutability::Const
        };

        if self.eat_kw("function") {
            let first = self.expect_ident()?;

            if self.is_op(".") || self.is_op(":") {
                let mut target = Expr::Name(first);
                let mut is_method = false;
                loop {
                    if self.eat_op(".") {
                        let field = self.expect_ident()?;
                        target = Expr::Index { base: Box::new(target), key: Box::new(Expr::Str(field)) };
                    } else if self.is_op(":") {
                        self.bump();
                        let field = self.expect_ident()?;
                        target = Expr::Index { base: Box::new(target), key: Box::new(Expr::Str(field)) };
                        is_method = true;
                        break;
                    } else {
                        break;
                    }
                }
                let (mut params, is_vararg, body) = self.parse_function_rest()?;
                if is_method {
                    params.insert(0, "self".to_string());
                }
                let func = Expr::Function { name: String::new(), params, is_vararg, body };
                let lvalue = expr_to_lvalue(target)
                    .ok_or_else(|| self.error("invalid function declaration target"))?;
                return Ok(Stmt::Assign { targets: vec![lvalue], op: AssignOp::Assign, values: vec![func], line });
            }

            let (params, is_vararg, body) = self.parse_function_rest()?;
            return Ok(Stmt::Declare {
                visibility,
                mutability,
                names: vec![first.clone()],
                inits: vec![Expr::Function { name: first, params, is_vararg, body }],
                line,
            });
        }

        let mut names = vec![self.parse_typed_name()?];
        while self.eat_delim(",") {
            names.push(self.parse_typed_name()?);
        }
        self.expect_op("=")?;
        let inits = self.parse_expr_list()?;
        Ok(Stmt::Declare { visibility, mutability, names, inits, line })
    }

    fn modifier_then(&self, kw: &str) -> bool {
        let mut i = 0;
        while self.peek_at(i).kind == TokenKind::Identifier
            && matches!(self.peek_at(i).text.as_str(), "pub" | "final" | "abstract")
        {
            i += 1;
        }
        let t = self.peek_at(i);
        t.kind == TokenKind::Identifier && t.text == kw
    }

    fn parse_class(&mut self) -> PResult<Stmt> {
        let mut visibility = Visibility::Local;
        let mut is_final = false;
        let mut is_abstract = false;
        loop {
            if self.eat_kw("pub") {
                visibility = Visibility::Pub;
            } else if self.eat_kw("final") {
                is_final = true;
            } else if self.eat_kw("abstract") {
                is_abstract = true;
            } else {
                break;
            }
        }
        self.expect_kw("class")?;
        let name = self.expect_ident()?;
        self.skip_type_arguments();
        let parent = if self.eat_kw("extends") { Some(self.expect_ident()?) } else { None };
        let mut mixins = Vec::new();
        if self.eat_kw("mixin") {
            mixins.push(self.expect_ident()?);
            while self.eat_delim(",") {
                mixins.push(self.expect_ident()?);
            }
        }
        let mut interfaces = Vec::new();
        if self.eat_kw("implements") {
            interfaces.push(self.expect_ident()?);
            while self.eat_delim(",") {
                interfaces.push(self.expect_ident()?);
            }
        }
        self.expect_delim("{")?;
        let mut members = Vec::new();
        loop {
            while self.eat_delim(";") {}
            if self.is_delim("}") || self.at_eof() {
                break;
            }
            members.push(self.parse_class_member()?);
        }
        self.expect_delim("}")?;
        Ok(Stmt::Class { visibility, is_final, is_abstract, name, parent, mixins, interfaces, members })
    }

    fn parse_interface(&mut self) -> PResult<Stmt> {
        let visibility = if self.eat_kw("pub") { Visibility::Pub } else { Visibility::Local };
        self.expect_kw("interface")?;
        let name = self.expect_ident()?;
        let mut parents = Vec::new();
        if self.eat_kw("extends") {
            parents.push(self.expect_ident()?);
            while self.eat_delim(",") {
                parents.push(self.expect_ident()?);
            }
        }
        self.expect_delim("{")?;
        let mut members = Vec::new();
        loop {
            while self.eat_delim(";") {}
            if self.is_delim("}") || self.at_eof() {
                break;
            }

            self.eat_kw("function");
            let member = self.expect_ident()?;
            if self.is_delim("(") {
                self.skip_balanced("(", ")");
            }
            if self.eat_op(":") {
                let _ = self.parse_type()?;
            }
            members.push(member);
        }
        self.expect_delim("}")?;
        Ok(Stmt::Interface { visibility, name, parents, members })
    }

    fn parse_enum(&mut self) -> PResult<Stmt> {
        let line = self.peek().span.line;

        let visibility = if self.eat_kw("pub") || self.eat_kw("export") {
            Visibility::Pub
        } else {
            Visibility::Local
        };
        self.expect_kw("enum")?;
        let name = self.expect_ident()?;
        self.expect_delim("{")?;
        let mut variants = Vec::new();
        loop {
            while self.eat_delim(",") || self.eat_delim(";") {}
            if self.is_delim("}") || self.at_eof() {
                break;
            }
            let vname = self.expect_ident()?;
            let value = if self.eat_op("=") { Some(self.parse_expr()?) } else { None };
            variants.push((vname, value));
        }
        self.expect_delim("}")?;
        Ok(Stmt::Enum { visibility, name, variants, line })
    }

    fn skip_balanced(&mut self, open: &str, close: &str) {
        if !self.eat_delim(open) {
            return;
        }
        let mut depth = 1;
        while depth > 0 && !self.at_eof() {
            if self.is_delim(open) {
                depth += 1;
            } else if self.is_delim(close) {
                depth -= 1;
            }
            self.bump();
        }
    }

    fn parse_class_member(&mut self) -> PResult<ClassMember> {
        let access = if self.eat_kw("private") {
            Access::Private
        } else if self.eat_kw("protected") {
            Access::Protected
        } else {
            self.eat_kw("public");
            Access::Public
        };
        let is_static = self.eat_kw("static");
        let is_abstract = self.eat_kw("abstract");
        let is_final = self.eat_kw("final");
        let is_override = self.eat_kw("override");

        if self.eat_kw("constructor") {
            return Ok(ClassMember::Constructor { func: self.parse_fn_body()? });
        }
        if self.eat_kw("destructor") {
            return Ok(ClassMember::Destructor { func: self.parse_fn_body()? });
        }
        if self.eat_kw("operator") {
            let t = self.peek().clone();
            let symbol = match t.kind {
                TokenKind::Operator | TokenKind::Identifier => self.bump().text,
                _ => return Err(self.error("expected an operator symbol after `operator`")),
            };
            return Ok(ClassMember::Operator { symbol, func: self.parse_fn_body()? });
        }

        if self.is_kw("get") && self.peek2().kind == TokenKind::Identifier {
            self.bump();
            let name = self.expect_ident()?;
            return Ok(ClassMember::Getter { access, name, func: self.parse_fn_body()? });
        }
        if self.is_kw("set") && self.peek2().kind == TokenKind::Identifier {
            self.bump();
            let name = self.expect_ident()?;
            return Ok(ClassMember::Setter { access, name, func: self.parse_fn_body()? });
        }
        if self.eat_kw("function") {
            let name = self.expect_ident()?;
            let func = self.parse_fn_body()?;
            return Ok(ClassMember::Method { access, is_static, is_abstract, is_final, is_override, name, func });
        }

        let name = self.expect_ident()?;
        if self.eat_op(":") {
            let _ = self.parse_type()?;
        }
        let default = if self.eat_op("=") { Some(self.parse_expr()?) } else { None };
        Ok(ClassMember::Field { access, is_static, name, default })
    }

    fn parse_buff(&mut self) -> PResult<Stmt> {
        let line = self.peek().span.line;
        self.expect_kw("buff")?;
        let size_tok = self.bump();
        let digits: String = size_tok.text.chars().filter(|c| *c != '_').collect();
        let size: u64 = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
            .map(|h| u64::from_str_radix(h, 16))
            .unwrap_or_else(|| digits.parse())
            .map_err(|_| self.error("buff size must be a non-negative integer"))?;
        let name = self.expect_ident()?;
        if self.eat_op(":") {
            let _ = self.parse_type()?;
        }
        self.expect_op("=")?;
        let init = self.parse_expr()?;
        Ok(Stmt::Buff { name, size, init, line })
    }

    fn parse_type_alias(&mut self) -> PResult<Stmt> {
        self.expect_kw("type")?;
        let name = self.expect_ident()?;
        self.skip_type_arguments();
        self.expect_op("=")?;
        let ty = self.parse_type()?;
        Ok(Stmt::TypeAlias { name, ty })
    }

    fn parse_typed_name(&mut self) -> PResult<String> {
        let name = self.expect_ident()?;
        if self.eat_op(":") {
            let _ = self.parse_type()?;
        }
        Ok(name)
    }

    fn parse_while(&mut self) -> PResult<Stmt> {
        let line = self.peek().span.line;
        self.expect_kw("while")?;
        let cond = self.parse_expr()?;
        self.expect_kw("do")?;
        let body = self.parse_block(&["end"])?;
        self.expect_kw("end")?;
        Ok(Stmt::While { cond, body, line })
    }

    fn parse_for(&mut self) -> PResult<Stmt> {
        let line = self.peek().span.line;
        self.expect_kw("for")?;
        let first = self.expect_ident()?;
        if self.eat_op(":") {
            let _ = self.parse_type()?;
        }
        if self.is_op("=") {

            self.bump();
            let start = self.parse_expr()?;
            self.expect_delim(",")?;
            let stop = self.parse_expr()?;
            let step = if self.eat_delim(",") { Some(self.parse_expr()?) } else { None };
            self.expect_kw("do")?;
            let body = self.parse_block(&["end"])?;
            self.expect_kw("end")?;
            Ok(Stmt::ForNumeric { var: first, start, stop, step, body, line })
        } else {

            let mut names = vec![first];
            while self.eat_delim(",") {
                names.push(self.parse_typed_name()?);
            }
            self.expect_kw("in")?;
            let iters = self.parse_expr_list()?;
            self.expect_kw("do")?;
            let body = self.parse_block(&["end"])?;
            self.expect_kw("end")?;
            Ok(Stmt::ForIn { names, iters, body, line })
        }
    }

    fn parse_expr_list(&mut self) -> PResult<Vec<Expr>> {
        let mut exprs = vec![self.parse_expr()?];
        while self.eat_delim(",") {
            exprs.push(self.parse_expr()?);
        }
        Ok(exprs)
    }

    fn at_block_end(&self) -> bool {
        let t = self.peek();
        t.kind == TokenKind::Eof
            || (t.kind == TokenKind::Delimiter && t.text == ";")
            || (t.kind == TokenKind::Identifier
                && matches!(t.text.as_str(), "end" | "else" | "elseif"))
    }

    fn parse_function_rest(&mut self) -> PResult<(Vec<String>, bool, Vec<Stmt>)> {
        self.skip_type_arguments();
        self.expect_delim("(")?;
        let mut params = Vec::new();
        let mut is_vararg = false;
        if !self.is_delim(")") {
            loop {
                if self.eat_op("...") {
                    is_vararg = true;
                    if self.eat_op(":") {
                        let _ = self.parse_type()?;
                    }
                    break;
                }
                if self.eat_op("...:") {
                    is_vararg = true;
                    let _ = self.parse_type()?;
                    break;
                }
                params.push(self.parse_typed_name()?);
                if !self.eat_delim(",") {
                    break;
                }
            }
        }
        self.expect_delim(")")?;
        if self.eat_op(":") {
            let _ = self.parse_type()?;
        }
        let body = self.parse_block(&["end"])?;
        self.expect_kw("end")?;
        Ok((params, is_vararg, body))
    }

    fn parse_fn_body(&mut self) -> PResult<FnBody> {
        let (params, is_vararg, body) = self.parse_function_rest()?;
        Ok(FnBody { params, is_vararg, body })
    }

    fn parse_if(&mut self) -> PResult<Stmt> {
        let line = self.peek().span.line;
        self.expect_kw("if")?;
        let mut branches = Vec::new();
        let cond = self.parse_expr()?;
        self.expect_kw("then")?;
        let body = self.parse_block(&["elseif", "else", "end"])?;
        branches.push((cond, body));

        while self.eat_kw("elseif") {
            let cond = self.parse_expr()?;
            self.expect_kw("then")?;
            let body = self.parse_block(&["elseif", "else", "end"])?;
            branches.push((cond, body));
        }

        let else_block = if self.eat_kw("else") {
            Some(self.parse_block(&["end"])?)
        } else {
            None
        };
        self.expect_kw("end")?;
        Ok(Stmt::If { branches, else_block, line })
    }

    fn parse_expr_or_assign(&mut self) -> PResult<Stmt> {
        let line = self.peek().span.line;
        let first = self.parse_expr()?;

        let annotated = self.eat_op(":");
        if annotated {
            let _ = self.parse_type()?;
        }

        if self.is_delim(",") {
            let mut targets =
                vec![expr_to_lvalue(first).ok_or_else(|| self.error("invalid assignment target"))?];
            while self.eat_delim(",") {
                let e = self.parse_expr()?;
                if self.eat_op(":") {
                    let _ = self.parse_type()?;
                }
                targets.push(expr_to_lvalue(e).ok_or_else(|| self.error("invalid assignment target"))?);
            }
            self.expect_op("=")?;
            let values = self.parse_expr_list()?;
            return Ok(Stmt::Assign { targets, op: AssignOp::Assign, values, line });
        }

        if annotated {
            self.expect_op("=")?;
            let target = expr_to_lvalue(first).ok_or_else(|| self.error("invalid assignment target"))?;
            let values = self.parse_expr_list()?;
            return Ok(Stmt::Assign { targets: vec![target], op: AssignOp::Assign, values, line });
        }

        if let Some(op) = self.peek_assign_op() {
            self.bump();
            let target = expr_to_lvalue(first).ok_or_else(|| self.error("invalid assignment target"))?;
            let values = if op == AssignOp::Assign {
                self.parse_expr_list()?
            } else {

                vec![self.parse_expr()?]
            };
            Ok(Stmt::Assign { targets: vec![target], op, values, line })
        } else {

            match &first {
                Expr::Call { .. } | Expr::MethodCall { .. } | Expr::Switch { .. } => {
                    Ok(Stmt::Expr(first, line))
                }
                _ => Err(self.error(
                    "this expression is not a statement — call it with `()` (or `\"text\"` / `{ table }`)",
                )),
            }
        }
    }

    fn peek_assign_op(&self) -> Option<AssignOp> {
        let t = self.peek();
        if t.kind != TokenKind::Operator {
            return None;
        }
        Some(match t.text.as_str() {
            "=" => AssignOp::Assign,
            "+=" => AssignOp::Add,
            "-=" => AssignOp::Sub,
            "*=" => AssignOp::Mul,
            "/=" => AssignOp::Div,
            "%=" => AssignOp::Mod,
            "..=" => AssignOp::Concat,
            _ => return None,
        })
    }

    fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_binary(0)
    }

    fn parse_binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut left = self.parse_unary()?;
        while let Some((kind, prec, right_assoc)) = self.binop_info() {
            if prec < min_prec {
                break;
            }
            self.bump();
            let next_min = if right_assoc { prec } else { prec + 1 };
            let right = self.parse_binary(next_min)?;
            left = match kind {
                OpKind::Bin(op) => Expr::Binary { op, lhs: Box::new(left), rhs: Box::new(right) },
                OpKind::Log(op) => Expr::Logical { op, lhs: Box::new(left), rhs: Box::new(right) },
            };
        }
        Ok(left)
    }

    fn binop_info(&self) -> Option<(OpKind, u8, bool)> {
        let t = self.peek();
        match t.kind {
            TokenKind::Identifier => match t.text.as_str() {
                "or" => Some((OpKind::Log(LogicalOp::Or), 1, false)),
                "and" => Some((OpKind::Log(LogicalOp::And), 2, false)),
                _ => None,
            },
            TokenKind::Operator => Some(match t.text.as_str() {
                "==" => (OpKind::Bin(BinOp::Eq), 3, false),
                "~=" => (OpKind::Bin(BinOp::Ne), 3, false),
                "<" => (OpKind::Bin(BinOp::Lt), 3, false),
                "<=" => (OpKind::Bin(BinOp::Le), 3, false),
                ">" => (OpKind::Bin(BinOp::Gt), 3, false),
                ">=" => (OpKind::Bin(BinOp::Ge), 3, false),
                ".." => (OpKind::Bin(BinOp::Concat), 4, true),
                "+" => (OpKind::Bin(BinOp::Add), 5, false),
                "-" => (OpKind::Bin(BinOp::Sub), 5, false),
                "*" => (OpKind::Bin(BinOp::Mul), 6, false),
                "/" => (OpKind::Bin(BinOp::Div), 6, false),
                "%" => (OpKind::Bin(BinOp::Mod), 6, false),
                _ => return None,
            }),
            _ => None,
        }
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        if self.is_op("-") {
            self.bump();
            return Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(self.parse_unary()?) });
        }
        if self.is_op("#") {
            self.bump();
            return Ok(Expr::Unary { op: UnaryOp::Len, expr: Box::new(self.parse_unary()?) });
        }
        if self.is_kw("not") {
            self.bump();
            return Ok(Expr::Unary { op: UnaryOp::Not, expr: Box::new(self.parse_unary()?) });
        }
        self.parse_power()
    }

    fn parse_power(&mut self) -> PResult<Expr> {
        let base = self.parse_postfix()?;
        if self.is_op("^") {
            self.bump();
            let exp = self.parse_unary()?;
            return Ok(Expr::Binary { op: BinOp::Pow, lhs: Box::new(base), rhs: Box::new(exp) });
        }
        Ok(base)
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.eat_delim("[") {
                let key = self.parse_expr()?;
                self.expect_delim("]")?;
                expr = Expr::Index { base: Box::new(expr), key: Box::new(key) };
            } else if self.is_op(".") {
                self.bump();
                let name = self.expect_ident()?;
                expr = Expr::Index { base: Box::new(expr), key: Box::new(Expr::Str(name)) };
            } else if self.is_delim("(") {
                self.bump();
                let mut args = Vec::new();
                while !self.is_delim(")") && !self.at_eof() {
                    args.push(self.parse_expr()?);
                    if !self.eat_delim(",") {
                        break;
                    }
                }
                self.expect_delim(")")?;
                expr = Expr::Call { callee: Box::new(expr), args };
            } else if self.is_op(":")
                && self.peek2().kind == TokenKind::Identifier
                && self.peek_at(2).kind == TokenKind::Delimiter
                && self.peek_at(2).text == "("
            {

                self.bump();
                let method = self.expect_ident()?;
                self.expect_delim("(")?;
                let mut args = Vec::new();
                while !self.is_delim(")") && !self.at_eof() {
                    args.push(self.parse_expr()?);
                    if !self.eat_delim(",") {
                        break;
                    }
                }
                self.expect_delim(")")?;
                expr = Expr::MethodCall { receiver: Box::new(expr), method, args };
            } else if self.peek().kind == TokenKind::Str {

                let arg = Expr::Str(self.bump().text);
                expr = Expr::Call { callee: Box::new(expr), args: vec![arg] };
            } else if self.peek().kind == TokenKind::InterpStr {
                let t = self.bump();
                let arg = parse_interp(&t.text, t.span.line)?;
                expr = Expr::Call { callee: Box::new(expr), args: vec![arg] };
            } else if self.is_delim("{") {

                let arg = self.parse_table()?;
                expr = Expr::Call { callee: Box::new(expr), args: vec![arg] };
            } else if self.is_op("::") {

                self.bump();
                let _ = self.parse_type()?;
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let t = self.peek().clone();
        match t.kind {
            TokenKind::Int => {
                self.bump();
                Ok(Expr::Int(parse_int(&t.text).map_err(|m| self.error(m))?))
            }
            TokenKind::Float => {
                self.bump();
                Ok(Expr::Float(parse_float(&t.text).map_err(|m| self.error(m))?))
            }
            TokenKind::Str => {
                self.bump();
                Ok(Expr::Str(t.text))
            }
            TokenKind::InterpStr => {
                self.bump();
                parse_interp(&t.text, t.span.line)
            }
            TokenKind::Identifier => match t.text.as_str() {
                "true" => {
                    self.bump();
                    Ok(Expr::Bool(true))
                }
                "false" => {
                    self.bump();
                    Ok(Expr::Bool(false))
                }
                "nil" => {
                    self.bump();
                    Ok(Expr::Nil)
                }
                "function" => {
                    self.bump();
                    let (params, is_vararg, body) = self.parse_function_rest()?;
                    Ok(Expr::Function { name: String::new(), params, is_vararg, body })
                }
                "switch" => self.parse_switch(),
                _ => {
                    self.bump();
                    Ok(Expr::Name(t.text))
                }
            },
            TokenKind::Operator if t.text == "..." => {
                self.bump();
                Ok(Expr::Vararg)
            }
            TokenKind::Delimiter if t.text == "(" => {
                self.bump();
                let inner = self.parse_expr()?;
                self.expect_delim(")")?;
                Ok(inner)
            }
            TokenKind::Delimiter if t.text == "{" => self.parse_table(),
            _ => Err(self.error(format!("unexpected token {:?}", t.text))),
        }
    }

    fn parse_switch(&mut self) -> PResult<Expr> {
        self.expect_kw("switch")?;
        self.expect_delim("(")?;
        let subject = self.parse_expr()?;
        self.expect_delim(")")?;

        let mut cases = Vec::new();
        let mut default = None;
        loop {
            if self.eat_kw("case") {
                let pattern = self.parse_expr()?;
                let body = self.parse_block(&["end"])?;
                self.expect_kw("end")?;
                cases.push(SwitchCase { pattern, body });
            } else if self.eat_kw("default") {
                let body = self.parse_block(&["end"])?;
                self.expect_kw("end")?;
                default = Some(body);
            } else {
                break;
            }
        }
        self.expect_kw("end")?;
        Ok(Expr::Switch { subject: Box::new(subject), cases, default })
    }

    fn parse_table(&mut self) -> PResult<Expr> {
        self.expect_delim("{")?;
        let mut entries = Vec::new();
        while !self.is_delim("}") && !self.at_eof() {
            let entry = if self.eat_delim("[") {

                let key = self.parse_expr()?;
                self.expect_delim("]")?;
                self.expect_op("=")?;
                let value = self.parse_expr()?;
                TableEntry::Keyed { key, value }
            } else if self.peek().kind == TokenKind::Identifier
                && self.peek2().kind == TokenKind::Operator
                && self.peek2().text == "="
            {

                let name = self.bump().text;
                self.bump();
                let value = self.parse_expr()?;
                TableEntry::Keyed { key: Expr::Str(name), value }
            } else {
                TableEntry::Positional(self.parse_expr()?)
            };
            entries.push(entry);

            if !self.eat_delim(",") && !self.eat_delim(";") {
                break;
            }
        }
        self.expect_delim("}")?;
        Ok(Expr::Table(entries))
    }

    fn parse_type(&mut self) -> PResult<Type> {
        let left = self.parse_type_union()?;

        if self.eat_op("->") {
            let ret = self.parse_type()?;
            return Ok(Type::Function { params: vec![left], ret: Box::new(ret) });
        }
        Ok(left)
    }

    fn parse_type_union(&mut self) -> PResult<Type> {
        let mut parts = vec![self.parse_type_intersection()?];
        while self.eat_op("|") {
            parts.push(self.parse_type_intersection()?);
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap() } else { Type::Union(parts) })
    }

    fn parse_type_intersection(&mut self) -> PResult<Type> {
        let mut parts = vec![self.parse_type_postfix()?];
        while self.eat_op("&") {
            parts.push(self.parse_type_postfix()?);
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap() } else { Type::Intersection(parts) })
    }

    fn parse_type_postfix(&mut self) -> PResult<Type> {
        let mut ty = self.parse_type_primary()?;
        while self.eat_op("?") {
            ty = Type::Optional(Box::new(ty));
        }
        Ok(ty)
    }

    fn parse_type_primary(&mut self) -> PResult<Type> {
        let t = self.peek().clone();
        match t.kind {
            TokenKind::Str => {
                self.bump();
                Ok(Type::Literal(t.text))
            }

            TokenKind::Int | TokenKind::Float => {
                self.bump();
                Ok(Type::Named(t.text))
            }
            TokenKind::Identifier => {
                self.bump();
                self.skip_type_arguments();

                let mut name = t.text;
                while self.is_op(".") && self.peek2().kind == TokenKind::Identifier {
                    self.bump();
                    name.push('.');
                    name.push_str(&self.bump().text);
                    self.skip_type_arguments();
                }
                Ok(Type::Named(name))
            }
            TokenKind::Delimiter if t.text == "{" => self.parse_type_table(),
            TokenKind::Delimiter if t.text == "(" => self.parse_type_paren(),
            _ => Err(self.error(format!("expected a type, found {:?}", t.text))),
        }
    }

    fn parse_type_paren(&mut self) -> PResult<Type> {
        self.expect_delim("(")?;
        let mut params = Vec::new();
        if !self.is_delim(")") {
            loop {
                if self.eat_op("...") {

                    if !self.is_delim(")") && !self.is_op("->") {
                        let _ = self.parse_type()?;
                    }
                    break;
                }

                if self.peek().kind == TokenKind::Identifier
                    && self.peek2().kind == TokenKind::Operator
                    && self.peek2().text == ":"
                {
                    self.bump();
                    self.bump();
                }
                params.push(self.parse_type()?);
                if !self.eat_delim(",") {
                    break;
                }
            }
        }
        self.expect_delim(")")?;
        if self.eat_op("->") {
            let ret = self.parse_type()?;
            Ok(Type::Function { params, ret: Box::new(ret) })
        } else {
            Ok(params.into_iter().next().unwrap_or(Type::Named("nil".into())))
        }
    }

    fn skip_type_arguments(&mut self) {
        if !(self.peek().kind == TokenKind::Operator && self.peek().text.starts_with('<')) {
            return;
        }
        let mut depth: i32 = 0;
        loop {
            let t = self.peek();
            match t.kind {
                TokenKind::Eof => break,
                TokenKind::Operator => {
                    for ch in t.text.chars() {
                        if ch == '<' {
                            depth += 1;
                        } else if ch == '>' {
                            depth -= 1;
                        }
                    }
                    self.bump();
                    if depth <= 0 {
                        break;
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn parse_type_table(&mut self) -> PResult<Type> {
        self.expect_delim("{")?;
        if self.eat_delim("}") {
            return Ok(Type::Table(Vec::new()));
        }

        if self.is_delim("[") {
            self.bump();
            let _key = self.parse_type()?;
            self.expect_delim("]")?;
            if !self.eat_op(":") {
                self.eat_op("=");
            }
            let val = self.parse_type()?;
            let _ = self.eat_delim(",") || self.eat_delim(";");
            self.expect_delim("}")?;
            return Ok(Type::Array(Box::new(val)));
        }

        let is_struct = self.peek().kind == TokenKind::Identifier
            && self.peek2().kind == TokenKind::Operator
            && matches!(self.peek2().text.as_str(), ":" | "=" | "?");
        if is_struct {
            let mut fields = Vec::new();
            while !self.is_delim("}") && !self.at_eof() {
                let name = self.expect_ident()?;
                self.eat_op("?");
                if !self.eat_op(":") {
                    self.expect_op("=")?;
                }
                let ty = self.parse_type()?;
                fields.push((name, ty));
                if !self.eat_delim(",") && !self.eat_delim(";") {
                    break;
                }
            }
            self.expect_delim("}")?;
            Ok(Type::Table(fields))
        } else {

            let elem = self.parse_type()?;
            let _ = self.eat_delim(",") || self.eat_delim(";");
            self.expect_delim("}")?;
            Ok(Type::Array(Box::new(elem)))
        }
    }
}

enum OpKind {
    Bin(BinOp),
    Log(LogicalOp),
}

fn expr_to_lvalue(expr: Expr) -> Option<LValue> {
    match expr {
        Expr::Name(n) => Some(LValue::Name(n)),
        Expr::Index { base, key } => Some(LValue::Index { base, key }),
        _ => None,
    }
}

fn parse_interp(raw: &str, line: u32) -> PResult<Expr> {
    let chars: Vec<char> = raw.chars().collect();
    let mut parts: Vec<Expr> = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            lit.push('\\');
            if i + 1 < chars.len() {
                lit.push(chars[i + 1]);
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if c == '{' {
            if !lit.is_empty() {
                parts.push(Expr::Str(crate::lexer::unescape(&std::mem::take(&mut lit))));
            }
            i += 1;
            let mut depth = 1;
            let mut src = String::new();
            while i < chars.len() && depth > 0 {
                let d = chars[i];
                match d {
                    '{' => {
                        depth += 1;
                        src.push(d);
                        i += 1;
                    }
                    '}' => {
                        depth -= 1;
                        if depth > 0 {
                            src.push(d);
                        }
                        i += 1;
                    }
                    '"' | '\'' => {
                        src.push(d);
                        i += 1;
                        while i < chars.len() {
                            if chars[i] == '\\' && i + 1 < chars.len() {
                                src.push(chars[i]);
                                src.push(chars[i + 1]);
                                i += 2;
                                continue;
                            }
                            src.push(chars[i]);
                            let end = chars[i] == d;
                            i += 1;
                            if end {
                                break;
                            }
                        }
                    }
                    _ => {
                        src.push(d);
                        i += 1;
                    }
                }
            }
            let inner = parse_interp_expr(&src, line)?;
            parts.push(Expr::Call {
                callee: Box::new(Expr::Name("tostring".to_string())),
                args: vec![inner],
            });
            continue;
        }
        lit.push(c);
        i += 1;
    }
    if !lit.is_empty() {
        parts.push(Expr::Str(crate::lexer::unescape(&lit)));
    }
    if parts.is_empty() {
        return Ok(Expr::Str(String::new()));
    }
    let mut iter = parts.into_iter();
    let mut acc = iter.next().unwrap();
    for p in iter {
        acc = Expr::Binary { op: BinOp::Concat, lhs: Box::new(acc), rhs: Box::new(p) };
    }
    Ok(acc)
}

fn parse_interp_expr(src: &str, line: u32) -> PResult<Expr> {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return Err(ParseError { message: "empty interpolation `{}`".into(), line, col: 0 });
    }
    let tokens = crate::lexer::tokenize(trimmed)
        .map_err(|e| ParseError { message: e.message, line, col: 0 })?;
    Parser::new(tokens).parse_expr()
}

fn parse_int(text: &str) -> Result<i64, String> {
    let cleaned = text.replace('_', "");
    if let Some(hex) = cleaned.strip_prefix("0x").or_else(|| cleaned.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).map_err(|_| format!("invalid hex integer {text:?}"))
    } else {
        cleaned.parse::<i64>().map_err(|_| format!("invalid integer {text:?}"))
    }
}

fn parse_float(text: &str) -> Result<f64, String> {
    text.replace('_', "").parse::<f64>().map_err(|_| format!("invalid float {text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn p(src: &str) -> Vec<Stmt> {
        parse(tokenize(src).unwrap()).unwrap()
    }

    #[test]
    fn parses_declarations() {
        let s = p("pub const x = 1 + 2");
        assert_eq!(
            s[0],
            Stmt::Declare {
                visibility: Visibility::Pub,
                mutability: Mutability::Const,
                names: vec!["x".into()],
                inits: vec![Expr::Binary {
                    op: BinOp::Add,
                    lhs: Box::new(Expr::Int(1)),
                    rhs: Box::new(Expr::Int(2)),
                }],
                line: 1,
            }
        );
    }

    #[test]
    fn multiple_assignment() {
        let s = p("local a, b = 1, 2");
        assert!(matches!(&s[0], Stmt::Declare { names, inits, .. } if names.len() == 2 && inits.len() == 2));
        let s2 = p("a, b = b, a");
        assert!(matches!(&s2[0], Stmt::Assign { targets, values, .. } if targets.len() == 2 && values.len() == 2));
    }

    #[test]
    fn precedence_and_parens() {

        let s = p("local x = (1 + 1) + 1");
        let Stmt::Declare { inits, .. } = &s[0] else { panic!("expected declare") };
        let Expr::Binary { op: BinOp::Add, lhs, .. } = &inits[0] else { panic!("expected add") };
        assert!(matches!(**lhs, Expr::Binary { op: BinOp::Add, .. }));
    }

    #[test]
    fn and_or_idiom() {
        let s = p("local t = a and b or c");

        let Stmt::Declare { inits, .. } = &s[0] else { panic!("expected declare") };
        let Expr::Logical { op: LogicalOp::Or, lhs, .. } = &inits[0] else { panic!("expected or") };
        assert!(matches!(**lhs, Expr::Logical { op: LogicalOp::And, .. }));
    }

    #[test]
    fn tables_and_indexing() {
        let s = p(r#"local t = {1, ["Test"] = true, name = 2}"#);
        let Stmt::Declare { inits, .. } = &s[0] else { panic!("expected declare") };
        assert!(matches!(&inits[0], Expr::Table(e) if e.len() == 3));
        let s2 = p("t[1] += 5");
        assert!(matches!(&s2[0], Stmt::Assign { op: AssignOp::Add, targets, .. } if matches!(targets[0], LValue::Index { .. })));
    }

    #[test]
    fn rich_type_syntax_parses() {
        for src in [
            "local x: number? = 1",
            "type Fn = (a: number, b: string) -> boolean",
            "type Arr = { number }",
            "type Map = { [string]: number }",
            "type S = { a: boolean, b: string? }",
            "export type T = { var = true }",
            "local y: A | B & C = nil",
            "local z = v :: { x: number } | nil",
        ] {
            assert!(parse(tokenize(src).unwrap()).is_ok(), "failed to parse: {src}");
        }
    }

    #[test]
    fn call_sugar_and_bare_expression() {

        assert!(parse(tokenize("print \"x\"").unwrap()).is_ok());
        assert!(parse(tokenize("print { 1, 2 }").unwrap()).is_ok());
        assert!(matches!(&p("print \"x\"")[0], Stmt::Expr(Expr::Call { .. }, _)));

        assert!(parse(tokenize("print").unwrap()).is_err());
        assert!(parse(tokenize("grag.test").unwrap()).is_err());

        assert!(parse(tokenize("obj:m()").unwrap()).is_ok());
        assert!(parse(tokenize("f(1)").unwrap()).is_ok());
    }

    #[test]
    fn loops_parse() {
        assert!(matches!(&p("while x do end")[0], Stmt::While { .. }));
        assert!(matches!(&p("for i = 1, 10 do end")[0], Stmt::ForNumeric { .. }));
        assert!(matches!(&p("for k, v in pairs(t) do end")[0], Stmt::ForIn { .. }));
        assert!(matches!(&p("break")[0], Stmt::Break { .. }));
    }
}
