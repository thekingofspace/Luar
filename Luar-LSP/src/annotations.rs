use crate::type_syntax::{TypeExpr, parse_type_prefix};
use luar::lexer::{Token, TokenKind};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnnotationSet {
    pub vars: HashMap<(String, u32), TypeExpr>,
    pub fn_returns: HashMap<(String, u32), TypeExpr>,
    pub fn_params: HashMap<(String, u32), HashMap<String, TypeExpr>>,
    pub fn_generics: HashMap<(String, u32), Vec<String>>,
    pub class_fields: HashMap<(String, String), TypeExpr>,
    pub method_returns: HashMap<(String, String), TypeExpr>,
    pub method_params: HashMap<(String, String), HashMap<String, TypeExpr>>,
    pub getter_returns: HashMap<(String, String), TypeExpr>,
    pub exported_types: HashSet<String>,
    pub alias_generics: HashMap<String, Vec<String>>,
    pub cast_vars: HashSet<(String, u32)>,
}

impl AnnotationSet {
    pub fn var(&self, name: &str, line: u32) -> Option<&TypeExpr> {
        self.vars.get(&(name.to_string(), line))
    }

    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
            && self.fn_returns.is_empty()
            && self.fn_params.is_empty()
            && self.fn_generics.is_empty()
            && self.class_fields.is_empty()
            && self.method_returns.is_empty()
            && self.method_params.is_empty()
            && self.getter_returns.is_empty()
            && self.exported_types.is_empty()
            && self.alias_generics.is_empty()
            && self.cast_vars.is_empty()
    }
}

const STOP_KEYWORDS: [&str; 22] = [
    "local", "const", "pub", "export", "function", "if", "elseif", "else", "while", "for",
    "return", "end", "class", "enum", "interface", "type", "do", "break", "switch", "buff",
    "freebuff", "in",
];

pub fn scan(src: &str) -> AnnotationSet {
    let Ok(tokens) = luar::tokenize(src) else {
        return AnnotationSet::default();
    };
    let toks: Vec<Token> = tokens
        .into_iter()
        .filter(|t| t.kind != TokenKind::Comment)
        .collect();
    let mut byte_of: Vec<usize> = src.char_indices().map(|(b, _)| b).collect();
    byte_of.push(src.len());
    let mut scanner = Scanner {
        src,
        toks,
        byte_of,
        i: 0,
        depth: 0,
        class_stack: Vec::new(),
        out: AnnotationSet::default(),
    };
    scanner.run();
    scanner.out
}

struct Scanner<'a> {
    src: &'a str,
    toks: Vec<Token>,
    byte_of: Vec<usize>,
    i: usize,
    depth: i32,
    class_stack: Vec<(i32, String)>,
    out: AnnotationSet,
}

impl<'a> Scanner<'a> {
    fn kind(&self, i: usize) -> TokenKind {
        self.toks
            .get(i)
            .map(|t| t.kind)
            .unwrap_or(TokenKind::Eof)
    }

    fn text(&self, i: usize) -> &str {
        self.toks.get(i).map(|t| t.text.as_str()).unwrap_or("")
    }

    fn line(&self, i: usize) -> u32 {
        self.toks.get(i).map(|t| t.span.line).unwrap_or(0)
    }

    fn is_ident(&self, i: usize, s: &str) -> bool {
        self.kind(i) == TokenKind::Identifier && self.text(i) == s
    }

    fn is_op(&self, i: usize, s: &str) -> bool {
        self.kind(i) == TokenKind::Operator && self.text(i) == s
    }

    fn is_delim(&self, i: usize, s: &str) -> bool {
        self.kind(i) == TokenKind::Delimiter && self.text(i) == s
    }

    fn byte_at(&self, i: usize) -> usize {
        let ci = self
            .toks
            .get(i)
            .map(|t| t.span.start)
            .unwrap_or(self.byte_of.len() - 1);
        self.byte_of[ci.min(self.byte_of.len() - 1)]
    }

    fn in_class_body(&self) -> Option<String> {
        match self.class_stack.last() {
            Some((d, name)) if *d == self.depth => Some(name.clone()),
            _ => None,
        }
    }

    fn run(&mut self) {
        while self.i < self.toks.len() {
            if self.kind(self.i) == TokenKind::Eof {
                break;
            }
            self.step();
        }
    }

    fn step(&mut self) {
        let i = self.i;
        match self.kind(i) {
            TokenKind::Delimiter => {
                match self.text(i) {
                    "(" | "{" | "[" => self.depth += 1,
                    ")" | "}" | "]" => {
                        self.depth -= 1;
                        while matches!(self.class_stack.last(), Some((d, _)) if *d > self.depth) {
                            self.class_stack.pop();
                        }
                    }
                    _ => {}
                }
                self.i += 1;
            }
            TokenKind::Identifier => self.ident_step(),
            _ => self.i += 1,
        }
    }

    fn ident_step(&mut self) {
        let i = self.i;
        let class_ctx = self.in_class_body();
        match self.text(i) {
            "export" | "pub" if class_ctx.is_none() => self.handle_modifiers(),
            "local" | "const" if class_ctx.is_none() => self.handle_modifiers(),
            "type" if class_ctx.is_none() && self.kind(i + 1) == TokenKind::Identifier => {
                self.handle_alias(false)
            }
            "function" => {
                let line = self.line(i);
                self.i += 1;
                self.handle_function(line, class_ctx);
            }
            "class" if self.kind(i + 1) == TokenKind::Identifier => self.handle_class_header(),
            "public" | "private" | "protected" | "static" | "abstract" | "final" | "override"
                if class_ctx.is_some() =>
            {
                self.handle_class_member(class_ctx.unwrap())
            }
            "constructor" if class_ctx.is_some() && self.is_delim(i + 1, "(") => {
                self.i += 1;
                let params = self.scan_params();
                if !params.is_empty() {
                    self.out
                        .method_params
                        .insert((class_ctx.unwrap(), "constructor".to_string()), params);
                }
            }
            "get" if class_ctx.is_some() && self.kind(i + 1) == TokenKind::Identifier => {
                let name = self.text(i + 1).to_string();
                self.i += 2;
                if self.is_delim(self.i, "(") {
                    let _ = self.scan_params();
                    if self.is_op(self.i, ":") {
                        self.i += 1;
                        if let Some(ty) = self.parse_type_here() {
                            self.out
                                .getter_returns
                                .insert((class_ctx.unwrap(), name), ty);
                        }
                    }
                }
            }
            _ => {
                if !self.try_bare_assignment(class_ctx.is_some()) {
                    self.i += 1;
                }
            }
        }
    }

    fn handle_modifiers(&mut self) {
        let kw_line = self.line(self.i);
        let mut saw_decl_keyword = false;
        loop {
            match self.text(self.i) {
                "export" | "pub" => self.i += 1,
                "local" | "const" => {
                    saw_decl_keyword = true;
                    self.i += 1;
                }
                _ => break,
            }
            if self.kind(self.i) != TokenKind::Identifier {
                return;
            }
        }
        match self.text(self.i) {
            "type" if self.kind(self.i + 1) == TokenKind::Identifier => {
                self.handle_alias(true);
            }
            "function" => {
                self.i += 1;
                self.handle_function(kw_line, None);
            }
            "class" | "enum" | "interface" | "buff" => {}
            _ if self.kind(self.i) == TokenKind::Identifier => {
                self.handle_name_list(kw_line, saw_decl_keyword || true);
            }
            _ => {}
        }
    }

    fn handle_alias(&mut self, exported_prefix: bool) {
        let mut exported = exported_prefix;
        if self.is_ident(self.i, "export") {
            exported = true;
            self.i += 1;
        }
        if !self.is_ident(self.i, "type") {
            return;
        }
        self.i += 1;
        if self.kind(self.i) != TokenKind::Identifier {
            return;
        }
        let name = self.text(self.i).to_string();
        self.i += 1;
        if exported {
            self.out.exported_types.insert(name.clone());
        }
        if self.kind(self.i) == TokenKind::Operator && self.text(self.i).starts_with('<') {
            let mut params = Vec::new();
            let mut balance: i32 = 0;
            while self.i < self.toks.len() {
                match self.kind(self.i) {
                    TokenKind::Operator => {
                        for c in self.text(self.i).chars() {
                            if c == '<' {
                                balance += 1;
                            } else if c == '>' {
                                balance -= 1;
                            }
                        }
                        self.i += 1;
                        if balance <= 0 {
                            break;
                        }
                    }
                    TokenKind::Identifier => {
                        if balance == 1 {
                            params.push(self.text(self.i).to_string());
                        }
                        self.i += 1;
                    }
                    TokenKind::Eof => break,
                    _ => self.i += 1,
                }
            }
            if !params.is_empty() {
                self.out.alias_generics.insert(name, params);
            }
        }
    }

    fn handle_name_list(&mut self, kw_line: u32, _declared: bool) {
        let mut names: Vec<(String, u32)> = Vec::new();
        let mut annotated = false;
        loop {
            if self.kind(self.i) != TokenKind::Identifier {
                break;
            }
            let name = self.text(self.i).to_string();
            let name_line = self.line(self.i);
            names.push((name.clone(), name_line));
            self.i += 1;
            if self.is_op(self.i, ":") {
                self.i += 1;
                if let Some(ty) = self.parse_type_here() {
                    annotated = true;
                    self.out
                        .vars
                        .insert((name.clone(), kw_line), ty.clone());
                    if name_line != kw_line {
                        self.out.vars.insert((name, name_line), ty);
                    }
                }
            }
            if self.is_delim(self.i, ",") {
                self.i += 1;
                continue;
            }
            break;
        }
        if self.is_op(self.i, "=") {
            self.i += 1;
            self.scan_init_for_cast(&names, kw_line, annotated);
        }
    }

    fn try_bare_assignment(&mut self, in_class: bool) -> bool {
        let i = self.i;
        if in_class {
            return false;
        }
        if i > 0 {
            let prev = &self.toks[i - 1];
            if prev.kind == TokenKind::Operator && (prev.text == "." || prev.text == ":") {
                return false;
            }
        }
        if self.kind(i) != TokenKind::Identifier
            || STOP_KEYWORDS.contains(&self.text(i))
        {
            return false;
        }
        if self.is_op(i + 1, ":") {
            let save = self.i;
            let name = self.text(i).to_string();
            let line = self.line(i);
            self.i = i + 2;
            if let Some(ty) = self.parse_type_here() {
                if self.is_op(self.i, "=") || self.is_delim(self.i, ",") {
                    self.out.vars.insert((name, line), ty);
                    return true;
                }
            }
            self.i = save + 1;
            return true;
        }
        if self.is_op(i + 1, "=") {
            let name = self.text(i).to_string();
            let line = self.line(i);
            self.i = i + 2;
            self.scan_init_for_cast(&[(name, line)], line, false);
            return true;
        }
        false
    }

    fn scan_init_for_cast(&mut self, names: &[(String, u32)], kw_line: u32, annotated: bool) {
        let mut ldepth: i32 = 0;
        let mut cast_ty: Option<TypeExpr> = None;
        while self.i < self.toks.len() {
            let i = self.i;
            match self.kind(i) {
                TokenKind::Eof => break,
                TokenKind::Delimiter => match self.text(i) {
                    "(" | "{" | "[" => {
                        ldepth += 1;
                        self.i += 1;
                    }
                    ")" | "}" | "]" => {
                        if ldepth == 0 {
                            break;
                        }
                        ldepth -= 1;
                        self.i += 1;
                    }
                    ";" if ldepth == 0 => break,
                    _ => self.i += 1,
                },
                TokenKind::Identifier if ldepth == 0 => {
                    if STOP_KEYWORDS.contains(&self.text(i)) {
                        break;
                    }
                    if self.is_op(i + 1, "=") && i > 0 && !self.prev_continues_expr(i) {
                        break;
                    }
                    if self.is_op(i + 1, ":")
                        && i > 0
                        && !self.prev_continues_expr(i)
                        && self.annotation_followed_by_eq(i)
                    {
                        break;
                    }
                    self.i += 1;
                }
                TokenKind::Operator if ldepth == 0 && self.text(i) == "::" => {
                    self.i += 1;
                    cast_ty = self.parse_type_here();
                }
                _ => self.i += 1,
            }
        }
        if let (Some(ty), false, 1) = (cast_ty, annotated, names.len()) {
            let (name, name_line) = &names[0];
            self.out.vars.insert((name.clone(), kw_line), ty.clone());
            self.out.cast_vars.insert((name.clone(), kw_line));
            if *name_line != kw_line {
                self.out.vars.insert((name.clone(), *name_line), ty);
                self.out.cast_vars.insert((name.clone(), *name_line));
            }
        }
    }

    fn annotation_followed_by_eq(&self, name_idx: usize) -> bool {
        let start_tok = name_idx + 2;
        if start_tok >= self.toks.len() || self.kind(start_tok) == TokenKind::Eof {
            return false;
        }
        let site = self.byte_at(start_tok);
        match parse_type_prefix(&self.src[site..]) {
            Ok((_, consumed)) => {
                let end = site + consumed;
                let mut j = start_tok;
                while j < self.toks.len()
                    && self.kind(j) != TokenKind::Eof
                    && self.byte_at(j) < end
                {
                    j += 1;
                }
                self.is_op(j, "=") || self.is_delim(j, ",")
            }
            Err(_) => false,
        }
    }

    fn prev_continues_expr(&self, i: usize) -> bool {
        if i == 0 {
            return false;
        }
        let prev = &self.toks[i - 1];
        match prev.kind {
            TokenKind::Operator => prev.text != "=",
            TokenKind::Delimiter => !matches!(prev.text.as_str(), ")" | "}" | "]"),
            _ => false,
        }
    }

    fn handle_function(&mut self, kw_line: u32, class_ctx: Option<String>) {
        if self.kind(self.i) != TokenKind::Identifier {
            self.skip_anonymous_function_header(kw_line);
            return;
        }
        let name = self.text(self.i).to_string();
        let name_line = self.line(self.i);
        self.i += 1;
        if self.kind(self.i) == TokenKind::Operator
            && (self.text(self.i) == "." || self.text(self.i) == ":")
        {
            return;
        }
        let generics = self.collect_generics();
        if !generics.is_empty() && class_ctx.is_none() {
            self.out
                .fn_generics
                .insert((name.clone(), kw_line), generics.clone());
            if name_line != kw_line {
                self.out
                    .fn_generics
                    .insert((name.clone(), name_line), generics);
            }
        }
        if !self.is_delim(self.i, "(") {
            return;
        }
        let params = self.scan_params();
        let ret = if self.is_op(self.i, ":") {
            self.i += 1;
            self.parse_type_here()
        } else {
            None
        };
        match class_ctx {
            Some(class) => {
                if !params.is_empty() {
                    self.out
                        .method_params
                        .insert((class.clone(), name.clone()), params);
                }
                if let Some(r) = ret {
                    self.out.method_returns.insert((class, name), r);
                }
            }
            None => {
                if !params.is_empty() {
                    self.out
                        .fn_params
                        .insert((name.clone(), kw_line), params.clone());
                    if name_line != kw_line {
                        self.out.fn_params.insert((name.clone(), name_line), params);
                    }
                }
                if let Some(r) = ret {
                    self.out
                        .fn_returns
                        .insert((name.clone(), kw_line), r.clone());
                    if name_line != kw_line {
                        self.out.fn_returns.insert((name, name_line), r);
                    }
                }
            }
        }
    }

    fn skip_anonymous_function_header(&mut self, _kw_line: u32) {
        self.skip_generics();
        if self.is_delim(self.i, "(") {
            let _ = self.scan_params();
            if self.is_op(self.i, ":") {
                self.i += 1;
                let _ = self.parse_type_here();
            }
        }
    }

    fn skip_generics(&mut self) {
        let _ = self.collect_generics();
    }

    fn collect_generics(&mut self) -> Vec<String> {
        if self.kind(self.i) != TokenKind::Operator || !self.text(self.i).starts_with('<') {
            return Vec::new();
        }
        let mut params = Vec::new();
        let mut balance: i32 = 0;
        while self.i < self.toks.len() {
            if self.kind(self.i) == TokenKind::Eof {
                break;
            }
            match self.kind(self.i) {
                TokenKind::Operator => {
                    for c in self.text(self.i).chars() {
                        if c == '<' {
                            balance += 1;
                        } else if c == '>' {
                            balance -= 1;
                        }
                    }
                    self.i += 1;
                    if balance <= 0 {
                        break;
                    }
                }
                TokenKind::Identifier => {
                    if balance == 1 {
                        params.push(self.text(self.i).to_string());
                    }
                    self.i += 1;
                }
                _ => self.i += 1,
            }
        }
        params
    }

    fn scan_params(&mut self) -> HashMap<String, TypeExpr> {
        let mut params = HashMap::new();
        if !self.is_delim(self.i, "(") {
            return params;
        }
        self.depth += 1;
        self.i += 1;
        loop {
            if self.kind(self.i) == TokenKind::Eof {
                break;
            }
            if self.is_delim(self.i, ")") {
                self.depth -= 1;
                self.i += 1;
                break;
            }
            if self.kind(self.i) == TokenKind::Identifier {
                let pname = self.text(self.i).to_string();
                self.i += 1;
                if self.is_op(self.i, ":") {
                    self.i += 1;
                    if let Some(ty) = self.parse_type_here() {
                        params.insert(pname, ty);
                    }
                }
            } else {
                self.i += 1;
            }
            if self.is_delim(self.i, ",") {
                self.i += 1;
            }
        }
        params
    }

    fn handle_class_header(&mut self) {
        let name = self.text(self.i + 1).to_string();
        self.i += 2;
        self.skip_generics();
        while self.i < self.toks.len() {
            if self.kind(self.i) == TokenKind::Eof {
                return;
            }
            if self.is_delim(self.i, "{") {
                self.depth += 1;
                self.class_stack.push((self.depth, name));
                self.i += 1;
                return;
            }
            if self.is_delim(self.i, ",") {
                self.i += 1;
                continue;
            }
            if self.kind(self.i) == TokenKind::Delimiter || self.is_ident(self.i, "end") {
                return;
            }
            self.i += 1;
        }
    }

    fn handle_class_member(&mut self, class: String) {
        while matches!(
            self.text(self.i),
            "public" | "private" | "protected" | "static" | "abstract" | "final" | "override"
        ) && self.kind(self.i) == TokenKind::Identifier
        {
            self.i += 1;
        }
        if self.is_ident(self.i, "function") {
            let line = self.line(self.i);
            self.i += 1;
            self.handle_function(line, Some(class));
            return;
        }
        if self.kind(self.i) == TokenKind::Identifier {
            let fname = self.text(self.i).to_string();
            if self.is_op(self.i + 1, ":") {
                self.i += 2;
                if let Some(ty) = self.parse_type_here() {
                    self.out.class_fields.insert((class, fname), ty);
                }
            } else {
                self.i += 1;
            }
        }
    }

    fn parse_type_here(&mut self) -> Option<TypeExpr> {
        let start_tok = self.i;
        if start_tok >= self.toks.len() || self.kind(start_tok) == TokenKind::Eof {
            return None;
        }
        let site = self.byte_at(start_tok);
        match parse_type_prefix(&self.src[site..]) {
            Ok((ty, consumed)) => {
                let end = site + consumed;
                let mut j = start_tok;
                while j < self.toks.len()
                    && self.kind(j) != TokenKind::Eof
                    && self.byte_at(j) < end
                {
                    j += 1;
                }
                self.i = j;
                Some(ty)
            }
            Err(_) => {
                self.i = start_tok + 1;
                None
            }
        }
    }
}
