
use std::collections::{HashMap, HashSet};

use crate::ast::{
    AssignOp, BinOp, ClassMember, Expr, LValue, Mutability, Stmt, SwitchCase, TableEntry, Visibility,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,

    pub code: &'static str,
    pub message: String,

    pub line: u32,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        write!(f, "{}:{} [{}] {}", self.line, kind, self.code, self.message)
    }
}

pub const CHECKS: &[&str] = &[
    "UnusedVariable",
    "UnusedParameter",
    "UnusedLoopVariable",
    "Redeclaration",
    "ShadowedVariable",
    "UnreachableCode",
    "EmptyBlock",
    "SelfAssignment",
    "MutateImmutable",
    "BreakOutsideLoop",
    "ConstantCondition",
    "RedundantBoolean",
    "DoubleNegation",
    "DuplicateClassMember",
    "DuplicateTableKey",
    "DuplicateCaseValue",
    "SelfComparison",
    "DivisionByZero",
    "AbstractMethodWithBody",
    "FinalAndAbstract",
    "EmptyClass",
];

pub fn check(source: &str) -> Vec<Diagnostic> {
    let directives = collect_directives(source);

    let tokens = match crate::lexer::tokenize(source) {
        Ok(t) => t,
        Err(e) => return vec![Diagnostic { severity: Severity::Error, code: "SyntaxError", message: e.to_string(), line: 0 }],
    };
    let program = match crate::parser::parse(tokens) {
        Ok(p) => p,
        Err(e) => return vec![Diagnostic { severity: Severity::Error, code: "SyntaxError", message: e.message, line: e.line }],
    };

    let mut f = Ferrite { scopes: Vec::new(), diags: Vec::new(), directives, current_line: 0, loop_depth: 0 };
    f.push_scope();
    f.walk_block(&program);
    f.pop_scope();
    f.diags.sort_by_key(|d| d.line);
    f.diags
}

#[derive(Default)]
pub struct Directives {
    global: HashSet<String>,
    per_line: HashMap<u32, HashSet<String>>,
}

impl Directives {
    pub fn silences(&self, code: &str, line: u32) -> bool {
        if self.global.contains(code) || self.global.contains("all") {
            return true;
        }
        self.per_line
            .get(&line)
            .is_some_and(|codes| codes.contains(code) || codes.contains("all"))
    }
}

pub fn collect_directives(source: &str) -> Directives {
    let mut d = Directives::default();
    let codes = |line: &str, idx: usize, kw: &str| -> Vec<String> {
        line[idx + kw.len()..]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };
    for (i, line) in source.lines().enumerate() {
        let ln = (i + 1) as u32;
        if let Some(idx) = line.find("--#disable-next-line") {
            d.per_line.entry(ln + 1).or_default().extend(codes(line, idx, "--#disable-next-line"));
        } else if let Some(idx) = line.find("--#disable-line") {
            d.per_line.entry(ln).or_default().extend(codes(line, idx, "--#disable-line"));
        } else if let Some(idx) = line.find("--#disable") {
            d.global.extend(codes(line, idx, "--#disable"));
        }
    }
    d
}

#[derive(PartialEq)]
enum Kind {
    Var,
    Param,
    Loop,
    Exempt,
}

struct Binding {
    mutable: bool,
    line: u32,
    used: bool,
    exported: bool,
    kind: Kind,
}

struct Ferrite {
    scopes: Vec<HashMap<String, Binding>>,
    diags: Vec<Diagnostic>,
    directives: Directives,
    current_line: u32,
    loop_depth: u32,
}

impl Ferrite {
    fn emit(&mut self, severity: Severity, code: &'static str, line: u32, message: String) {
        if self.directives.silences(code, line) {
            return;
        }
        self.diags.push(Diagnostic { severity, code, message, line });
    }

    fn warn(&mut self, code: &'static str, line: u32, message: String) {
        self.emit(Severity::Warning, code, line, message);
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            for (name, b) in scope {
                if b.used || b.exported || name.starts_with('_') {
                    continue;
                }
                match b.kind {
                    Kind::Var => self.warn("UnusedVariable", b.line, format!("variable '{name}' is declared but never used")),
                    Kind::Param => self.warn("UnusedParameter", b.line, format!("parameter '{name}' is never used")),
                    Kind::Loop => self.warn("UnusedLoopVariable", b.line, format!("loop variable '{name}' is never used")),
                    Kind::Exempt => {}
                }
            }
        }
    }

    fn declare(&mut self, name: &str, mutable: bool, exported: bool, line: u32, kind: Kind) {
        let in_current = self.scopes.last().is_some_and(|s| s.contains_key(name));
        let in_outer = !in_current && self.scopes.iter().rev().skip(1).any(|s| s.contains_key(name));
        if in_current && kind != Kind::Exempt {
            self.warn("Redeclaration", line, format!("variable '{name}' is redeclared in the same scope"));
        } else if in_outer && kind == Kind::Var {
            self.warn("ShadowedVariable", line, format!("variable '{name}' shadows one from an outer scope"));
        }

        let used = kind == Kind::Exempt;
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), Binding { mutable, line, used, exported, kind });
        }
    }

    fn use_name(&mut self, name: &str) -> Option<bool> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(b) = scope.get_mut(name) {
                b.used = true;
                return Some(!b.mutable);
            }
        }
        None
    }

    fn walk_block(&mut self, stmts: &[Stmt]) {
        let mut terminated = false;
        for s in stmts {
            if let Some(l) = stmt_line(s) {
                self.current_line = l;
            }
            if terminated {
                self.warn("UnreachableCode", self.current_line, "this code can never run (it follows a `return`/`break`)".into());
                terminated = false;
            }
            self.walk_stmt(s);
            if matches!(s, Stmt::Return { .. } | Stmt::Break { .. }) {
                terminated = true;
            }
        }
    }

    fn scoped_block(&mut self, stmts: &[Stmt], what: &'static str) {
        if stmts.is_empty() {
            self.warn("EmptyBlock", self.current_line, format!("empty {what} body"));
        }
        self.push_scope();
        self.walk_block(stmts);
        self.pop_scope();
    }

    fn walk_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Declare { visibility, mutability, names, inits, line } => {
                for e in inits {
                    self.walk_expr(e);
                }
                let mutable = matches!(mutability, Mutability::Mutable);
                let exported = matches!(visibility, Visibility::Pub);
                for name in names {
                    self.declare(name, mutable, exported, *line, Kind::Var);
                }
            }
            Stmt::Buff { name, init, line, .. } => {
                self.walk_expr(init);
                self.declare(name, true, false, *line, Kind::Var);
            }
            Stmt::FreeBuff { name, .. } => {
                self.use_name(name);
            }
            Stmt::Assign { targets, op, values, line } => {
                for v in values {
                    self.walk_expr(v);
                }

                if *op == AssignOp::Assign && targets.len() == 1 && values.len() == 1 {
                    if let (LValue::Name(t), Expr::Name(v)) = (&targets[0], &values[0]) {
                        if t == v {
                            self.warn("SelfAssignment", *line, format!("'{t}' is assigned to itself"));
                        }
                    }
                }
                for (i, t) in targets.iter().enumerate() {
                    match t {
                        LValue::Name(name) => {

                            let frees = *op == AssignOp::Assign && matches!(values.get(i), Some(Expr::Nil));
                            match self.use_name(name) {
                                Some(true) if !frees => {
                                    self.warn("MutateImmutable", *line, format!("cannot change immutable variable '{name}'"));
                                }
                                None if *op == AssignOp::Assign => self.declare(name, false, false, *line, Kind::Exempt),
                                _ => {}
                            }
                        }
                        LValue::Index { base, key } => {
                            self.walk_expr(base);
                            self.walk_expr(key);
                        }
                    }
                }
            }
            Stmt::Do(body) => self.scoped_block(body, "do"),
            Stmt::If { branches, else_block, line } => {
                self.current_line = *line;
                for (cond, body) in branches {
                    self.constant_condition(cond);
                    self.walk_expr(cond);
                    self.scoped_block(body, "if");
                }
                if let Some(body) = else_block {
                    self.scoped_block(body, "else");
                }
            }
            Stmt::While { cond, body, line } => {
                self.current_line = *line;
                self.constant_condition(cond);
                self.walk_expr(cond);
                self.loop_depth += 1;
                self.scoped_block(body, "while");
                self.loop_depth -= 1;
            }
            Stmt::ForNumeric { var, start, stop, step, body } => {
                self.walk_expr(start);
                self.walk_expr(stop);
                if let Some(s) = step {
                    self.walk_expr(s);
                }
                self.push_scope();
                self.declare(var, true, false, self.current_line, Kind::Loop);
                self.loop_depth += 1;
                self.walk_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }
            Stmt::ForIn { names, iters, body } => {
                for e in iters {
                    self.walk_expr(e);
                }
                self.push_scope();
                for name in names {
                    self.declare(name, true, false, self.current_line, Kind::Loop);
                }
                self.loop_depth += 1;
                self.walk_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }
            Stmt::Return { values, .. } => {
                for e in values {
                    self.walk_expr(e);
                }
            }
            Stmt::Break { line } => {
                if self.loop_depth == 0 {
                    self.emit(Severity::Error, "BreakOutsideLoop", *line, "`break` is not inside a loop".into());
                }
            }
            Stmt::Class { members, .. } => self.walk_class(members),
            Stmt::Enum { name, variants, line, .. } => {
                for (_, value) in variants {
                    if let Some(e) = value {
                        self.walk_expr(e);
                    }
                }

                self.declare(name, false, true, *line, Kind::Exempt);
            }
            Stmt::TypeAlias { .. } | Stmt::Interface { .. } => {}
            Stmt::Expr(e, _) => self.walk_expr(e),
        }
    }

    fn constant_condition(&mut self, cond: &Expr) {
        if let Expr::Bool(b) = cond {
            self.warn("ConstantCondition", self.current_line, format!("condition is always {b}"));
        }
    }

    fn walk_class(&mut self, members: &[ClassMember]) {
        if members.is_empty() {
            self.warn("EmptyClass", self.current_line, "class body is empty".into());
        }
        let mut seen: HashSet<String> = HashSet::new();
        for m in members {
            let name = match m {
                ClassMember::Field { name, .. } => Some(name.clone()),
                ClassMember::Method { name, .. } => Some(name.clone()),
                ClassMember::Getter { name, .. } => Some(format!("get {name}")),
                ClassMember::Setter { name, .. } => Some(format!("set {name}")),
                _ => None,
            };
            if let Some(n) = name {
                if !seen.insert(n.clone()) {
                    self.warn("DuplicateClassMember", self.current_line, format!("class member '{n}' is declared more than once"));
                }
            }
            match m {
                ClassMember::Field { default: Some(e), .. } => self.walk_expr(e),
                ClassMember::Field { .. } => {}
                ClassMember::Method { name, is_abstract, is_final, func, .. } => {
                    if *is_abstract && *is_final {
                        self.warn("FinalAndAbstract", self.current_line, format!("method '{name}' cannot be both `abstract` and `final`"));
                    }
                    if *is_abstract && !func.body.is_empty() {
                        self.warn("AbstractMethodWithBody", self.current_line, format!("abstract method '{name}' must not have a body"));
                    }
                    if !*is_abstract {
                        self.walk_fn(&func.params, &func.body);
                    }
                }
                ClassMember::Constructor { func }
                | ClassMember::Destructor { func }
                | ClassMember::Operator { func, .. }
                | ClassMember::Getter { func, .. }
                | ClassMember::Setter { func, .. } => self.walk_fn(&func.params, &func.body),
            }
        }
    }

    fn walk_fn(&mut self, params: &[String], body: &[Stmt]) {
        let saved_loop = self.loop_depth;
        self.loop_depth = 0;
        self.push_scope();
        for p in params {
            self.declare(p, true, false, self.current_line, Kind::Param);
        }
        self.walk_block(body);
        self.pop_scope();
        self.loop_depth = saved_loop;
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Name(name) => {
                self.use_name(name);
            }
            Expr::Index { base, key } => {
                self.walk_expr(base);
                self.walk_expr(key);
            }
            Expr::Call { callee, args } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.walk_expr(receiver);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Function { params, body, .. } => self.walk_fn(params, body),
            Expr::Table(entries) => {
                let mut seen: HashSet<String> = HashSet::new();
                for e in entries {
                    match e {
                        TableEntry::Positional(v) => self.walk_expr(v),
                        TableEntry::Keyed { key, value } => {
                            if let Some(k) = literal_key(key) {
                                if !seen.insert(k.clone()) {
                                    self.warn("DuplicateTableKey", self.current_line, format!("table key {k} is set more than once"));
                                }
                            }
                            self.walk_expr(key);
                            self.walk_expr(value);
                        }
                    }
                }
            }
            Expr::Switch { subject, cases, default } => {
                self.walk_expr(subject);
                let mut seen: HashSet<String> = HashSet::new();
                for SwitchCase { pattern, body } in cases {
                    if let Some(k) = literal_key(pattern) {
                        if !seen.insert(k.clone()) {
                            self.warn("DuplicateCaseValue", self.current_line, format!("duplicate `case` value {k}"));
                        }
                    }
                    self.walk_expr(pattern);
                    self.scoped_block(body, "case");
                }
                if let Some(body) = default {
                    self.scoped_block(body, "default");
                }
            }
            Expr::Unary { op, expr } => {
                if matches!(op, crate::ast::UnaryOp::Not) {
                    if let Expr::Unary { op: crate::ast::UnaryOp::Not, .. } = expr.as_ref() {
                        self.warn("DoubleNegation", self.current_line, "double negation `not not x` is redundant".into());
                    }
                }
                self.walk_expr(expr);
            }
            Expr::Binary { op, lhs, rhs } => {
                if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
                    if let (Expr::Name(a), Expr::Name(b)) = (lhs.as_ref(), rhs.as_ref()) {
                        if a == b {
                            self.warn("SelfComparison", self.current_line, format!("'{a}' is compared with itself"));
                        }
                    }
                }

                if matches!(op, BinOp::Eq | BinOp::Ne)
                    && (matches!(lhs.as_ref(), Expr::Bool(_)) || matches!(rhs.as_ref(), Expr::Bool(_)))
                {
                    self.warn("RedundantBoolean", self.current_line, "comparing with a boolean literal is redundant".into());
                }

                if matches!(op, BinOp::Div | BinOp::Mod) && is_zero(rhs) {
                    self.warn("DivisionByZero", self.current_line, "division or modulo by zero".into());
                }
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::Logical { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::Nil | Expr::Bool(_) | Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Vararg => {}
        }
    }
}

fn stmt_line(s: &Stmt) -> Option<u32> {
    match s {
        Stmt::Declare { line, .. }
        | Stmt::Assign { line, .. }
        | Stmt::Return { line, .. }
        | Stmt::Break { line }
        | Stmt::If { line, .. }
        | Stmt::While { line, .. }
        | Stmt::Buff { line, .. }
        | Stmt::FreeBuff { line, .. } => Some(*line),
        Stmt::Expr(_, line) => Some(*line),
        _ => None,
    }
}

fn literal_key(e: &Expr) -> Option<String> {
    match e {
        Expr::Int(i) => Some(format!("{i}")),
        Expr::Float(x) => Some(format!("{x}")),
        Expr::Bool(b) => Some(format!("{b}")),
        Expr::Str(s) => Some(format!("{s:?}")),
        _ => None,
    }
}

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Int(0)) || matches!(e, Expr::Float(x) if *x == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(src: &str, code: &str) -> bool {
        check(src).iter().any(|d| d.code == code)
    }

    #[test]
    fn flags_mutating_immutable() {
        let d = check("const x = 1\nx = 2");
        assert!(d.iter().any(|d| d.code == "MutateImmutable" && d.line == 2));
    }

    #[test]
    fn nil_frees_an_immutable() {

        assert!(!has("const x = 1\nx = nil", "MutateImmutable"));
        assert!(has("const x = 1\nx = 2", "MutateImmutable"));
    }

    #[test]
    fn allows_mutable_reassignment() {
        assert!(!has("local x = 1\nx = 2\nprint(x)", "MutateImmutable"));
    }

    #[test]
    fn flags_unused_and_respects_underscore() {
        assert!(has("local unusedThing = 5", "UnusedVariable"));
        assert!(check("local _ignored = 5").is_empty());
    }

    #[test]
    fn directive_disables_check() {
        assert!(!has("--#disable MutateImmutable\nconst x = 1\nx = 2\nprint(x)", "MutateImmutable"));
        assert!(!has("--#disable all\nconst x = 1\nx = 2", "MutateImmutable"));
    }

    #[test]
    fn constant_condition_points_at_the_if() {
        let d = check("local a = 1\nlocal b = 2\nif true then\n  print(a + b)\nend");
        let cc = d.iter().find(|d| d.code == "ConstantCondition").expect("expected ConstantCondition");
        assert_eq!(cc.line, 3, "should point at the `if`, not the line above it");
    }

    #[test]
    fn new_checks() {
        assert!(has("for i = 1, 10 do print(1) end", "UnusedLoopVariable"));
        assert!(!has("for i = 1, 10 do print(i) end", "UnusedLoopVariable"));
        assert!(!has("for _ = 1, 10 do print(1) end", "UnusedLoopVariable"));
        assert!(has("local x = true\nprint(x == true)", "RedundantBoolean"));
        assert!(has("local x = true\nprint(not not x)", "DoubleNegation"));
        assert!(has("class C { abstract function f() return 1 end }", "AbstractMethodWithBody"));
        assert!(!has("class C { abstract function f() end\nfunction g() return 1 end }", "AbstractMethodWithBody"));
        assert!(has("class C {}", "EmptyClass"));
    }

    #[test]
    fn per_line_directives() {
        assert!(!has("const x = 1\nx = 2 --#disable-line MutateImmutable\nprint(x)", "MutateImmutable"));
        assert!(!has("const x = 1\n--#disable-next-line MutateImmutable\nx = 2\nprint(x)", "MutateImmutable"));

        assert!(has("const x = 1\nx = 2 --#disable-line UnusedVariable\nprint(x)", "MutateImmutable"));
    }

    #[test]
    fn many_checks() {
        assert!(has("break", "BreakOutsideLoop"));
        assert!(has("if true then print(1) end", "ConstantCondition"));
        assert!(has("local x = 1\nx = x\nprint(x)", "SelfAssignment"));
        assert!(has("local x = 1\nprint(x == x)", "SelfComparison"));
        assert!(has("local x = 1 / 0\nprint(x)", "DivisionByZero"));
        assert!(has("local t = { a = 1, a = 2 }\nprint(t)", "DuplicateTableKey"));
        assert!(has("local function f() return 1\nprint(2) end\nprint(f())", "UnreachableCode"));
        assert!(has("do end", "EmptyBlock"));
        assert!(has("local function f(p) return 1 end\nprint(f(0))", "UnusedParameter"));
        assert!(has("class C { x = 1\nx = 2 }", "DuplicateClassMember"));
    }
}
