use crate::annotations::AnnotationSet;
use crate::infer::Analysis;
use crate::type_syntax::{NameSeg, Param, TableField, TableTypeExpr, TypeAlias, TypeExpr};
use crate::types::{FunctionType, ParamInfo, TableType, Type};
use luar::ast::{ClassMember, Expr, Stmt, TableEntry};
use std::collections::{HashMap, HashSet};

const MAX_RESOLVE_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    Basic(Type),
    Class(String),
    Enum(String),
    Interface(String),
    StringLiteral(String),
    NumberLiteral(String),
    Structural(TypeExpr),
    Unresolved(String),
}

#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    pub aliases: HashMap<String, TypeAlias>,
    pub classes: HashSet<String>,
    pub enums: HashSet<String>,
    pub interfaces: HashSet<String>,
    pub modules: HashMap<String, TypeEnv>,
}

pub fn from_luar_type(ty: &luar::ast::Type) -> TypeExpr {
    match ty {
        luar::ast::Type::Named(name) => {
            let first = name.chars().next().unwrap_or('_');
            if first.is_ascii_digit() || first == '-' {
                return TypeExpr::NumberLit(name.clone());
            }
            TypeExpr::Named(
                name.split('.')
                    .map(|seg| NameSeg {
                        name: seg.to_string(),
                        args: None,
                    })
                    .collect(),
            )
        }
        luar::ast::Type::Literal(s) => TypeExpr::StringLit(s.clone()),
        luar::ast::Type::Table(fields) => {
            if fields.is_empty() {
                TypeExpr::Table(TableTypeExpr::Empty)
            } else {
                TypeExpr::Table(TableTypeExpr::Record(
                    fields
                        .iter()
                        .map(|(name, t)| TableField {
                            name: name.clone(),
                            optional: false,
                            ty: from_luar_type(t),
                        })
                        .collect(),
                ))
            }
        }
        luar::ast::Type::Array(elem) => {
            TypeExpr::Table(TableTypeExpr::Array(Box::new(from_luar_type(elem))))
        }
        luar::ast::Type::Optional(inner) => TypeExpr::Optional(Box::new(from_luar_type(inner))),
        luar::ast::Type::Function { params, ret } => TypeExpr::Function {
            params: params
                .iter()
                .map(|p| Param::Positional {
                    name: None,
                    ty: from_luar_type(p),
                })
                .collect(),
            ret: Box::new(from_luar_type(ret)),
        },
        luar::ast::Type::Union(parts) => {
            TypeExpr::Union(parts.iter().map(from_luar_type).collect())
        }
        luar::ast::Type::Intersection(parts) => {
            TypeExpr::Intersection(parts.iter().map(from_luar_type).collect())
        }
    }
}

fn basic_value_type(name: &str) -> Option<Type> {
    match name {
        "number" | "integer" | "double" | "float" => Some(Type::Number),
        "string" => Some(Type::String),
        "boolean" | "bool" | "true" | "false" => Some(Type::Boolean),
        "nil" | "void" => Some(Type::Nil),
        "thread" => Some(Type::Thread),
        "table" => Some(Type::Table(TableType {
            name: Some("table".to_string()),
            ..TableType::default()
        })),
        "function" => Some(Type::Function(None)),
        "class" => Some(Type::Class(String::new())),
        "enum" => Some(Type::Enum(String::new())),
        "any" | "unknown" | "never" => Some(Type::Unknown),
        _ => None,
    }
}

struct Guard {
    seen: HashSet<String>,
    depth: usize,
}

impl Guard {
    fn new() -> Guard {
        Guard {
            seen: HashSet::new(),
            depth: 0,
        }
    }

    fn too_deep(&self) -> bool {
        self.depth > MAX_RESOLVE_DEPTH
    }
}

impl TypeEnv {
    pub fn from_program(stmts: &[Stmt]) -> TypeEnv {
        crate::scoped_large_stack(|| {
            let mut env = TypeEnv::default();
            env.collect_stmts(stmts);
            env
        })
    }

    pub fn from_source(src: &str) -> Result<TypeEnv, String> {
        let program = crate::parse_source_safe(src)?;
        Ok(TypeEnv::from_program(&program))
    }

    pub fn from_analysis(analysis: &Analysis) -> TypeEnv {
        let mut env = TypeEnv::default();
        for (name, ty) in &analysis.aliases {
            env.aliases.insert(
                name.clone(),
                TypeAlias {
                    exported: false,
                    name: name.clone(),
                    generics: Vec::new(),
                    ty: from_luar_type(ty),
                },
            );
        }
        env.classes.extend(analysis.classes.keys().cloned());
        env.enums.extend(analysis.enums.keys().cloned());
        env.interfaces.extend(analysis.interfaces.keys().cloned());
        env
    }

    pub fn apply_annotations(&mut self, ann: &AnnotationSet) {
        for name in &ann.exported_types {
            if let Some(alias) = self.aliases.get_mut(name) {
                alias.exported = true;
            }
        }
        for (name, generics) in &ann.alias_generics {
            if let Some(alias) = self.aliases.get_mut(name) {
                alias.generics = generics.clone();
            }
        }
    }

    pub fn merge_globals(&mut self, other: &TypeEnv) {
        for (name, alias) in &other.aliases {
            self.aliases
                .entry(name.clone())
                .or_insert_with(|| alias.clone());
        }
        self.classes.extend(other.classes.iter().cloned());
        self.enums.extend(other.enums.iter().cloned());
        self.interfaces.extend(other.interfaces.iter().cloned());
    }

    pub fn define_alias(&mut self, alias: TypeAlias) {
        self.aliases.insert(alias.name.clone(), alias);
    }

    pub fn define_class(&mut self, name: &str) {
        self.classes.insert(name.to_string());
    }

    pub fn define_enum(&mut self, name: &str) {
        self.enums.insert(name.to_string());
    }

    pub fn define_interface(&mut self, name: &str) {
        self.interfaces.insert(name.to_string());
    }

    pub fn define_module(&mut self, local_name: &str, env: TypeEnv) {
        self.modules.insert(local_name.to_string(), env);
    }

    pub fn exported_type_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .aliases
            .values()
            .filter(|a| a.exported)
            .map(|a| a.name.clone())
            .collect();
        names.extend(self.classes.iter().cloned());
        names.extend(self.enums.iter().cloned());
        names.sort();
        names.dedup();
        names
    }

    pub fn type_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.aliases.keys().cloned().collect();
        names.extend(self.classes.iter().cloned());
        names.extend(self.enums.iter().cloned());
        names.extend(self.interfaces.iter().cloned());
        names.sort();
        names.dedup();
        names
    }

    pub fn resolve_name(&self, name: &str) -> Resolved {
        let expr = TypeExpr::named(name);
        self.resolve_inner(&expr, &mut Guard::new())
    }

    pub fn resolve(&self, ty: &TypeExpr) -> Resolved {
        self.resolve_inner(ty, &mut Guard::new())
    }

    fn resolve_inner(&self, ty: &TypeExpr, guard: &mut Guard) -> Resolved {
        guard.depth += 1;
        if guard.too_deep() {
            guard.depth -= 1;
            return Resolved::Unresolved("<too deep>".to_string());
        }
        let result = self.resolve_dispatch(ty, guard);
        guard.depth -= 1;
        result
    }

    fn keys_union(&self, arg: &TypeExpr, guard: &mut Guard) -> Option<TypeExpr> {
        match self.resolve_inner(arg, guard) {
            Resolved::Structural(TypeExpr::Table(TableTypeExpr::Record(fields))) => {
                let lits: Vec<TypeExpr> = fields
                    .iter()
                    .map(|f| TypeExpr::StringLit(f.name.clone()))
                    .collect();
                match lits.len() {
                    0 => None,
                    1 => Some(lits.into_iter().next().unwrap()),
                    _ => Some(TypeExpr::Union(lits)),
                }
            }
            _ => None,
        }
    }

    fn value_of_key(
        &self,
        target: &TypeExpr,
        key: &TypeExpr,
        guard: &mut Guard,
    ) -> Option<TypeExpr> {
        let fields = match self.resolve_inner(target, guard) {
            Resolved::Structural(TypeExpr::Table(TableTypeExpr::Record(fields))) => fields,
            _ => return None,
        };
        let keys: Vec<String> = match self.resolve_inner(key, guard) {
            Resolved::StringLiteral(s) => vec![s],
            Resolved::Structural(TypeExpr::Union(parts)) => parts
                .iter()
                .filter_map(|p| match p {
                    TypeExpr::StringLit(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let matched: Vec<TypeExpr> = fields
            .iter()
            .filter(|f| keys.contains(&f.name))
            .map(|f| f.ty.clone())
            .collect();
        match matched.len() {
            0 => None,
            1 => Some(matched.into_iter().next().unwrap()),
            _ => Some(TypeExpr::Union(matched)),
        }
    }

    fn to_basic(&self, arg: &TypeExpr, guard: &mut Guard) -> Option<TypeExpr> {
        if matches!(arg.simple_name(), Some("true") | Some("false")) {
            return Some(TypeExpr::named("boolean"));
        }
        let value = self.value_type_inner(arg, guard);
        basic_expr_of(&value).or_else(|| Some(arg.clone()))
    }

    fn type_function(&self, seg: &NameSeg, guard: &mut Guard) -> Option<TypeExpr> {
        let args = seg.args.as_ref()?;
        match (seg.name.as_str(), args.len()) {
            ("keyof" | "KeyOf", 1) => self.keys_union(&args[0], guard),
            ("valueof" | "ValueOf", 2) => self.value_of_key(&args[0], &args[1], guard),
            ("tobasic" | "ToBasic", 1) => self.to_basic(&args[0], guard),
            ("notnil" | "NotNil", 1) => Some(self.strip_nil_resolved(&args[0], guard)),
            _ => None,
        }
    }

    fn strip_nil_resolved(&self, arg: &TypeExpr, guard: &mut Guard) -> TypeExpr {
        guard.depth += 1;
        if guard.too_deep() {
            guard.depth -= 1;
            return arg.clone();
        }
        let result = match arg {
            TypeExpr::Optional(inner) => self.strip_nil_resolved(inner, guard),
            TypeExpr::Union(parts) => {
                let kept: Vec<TypeExpr> = parts
                    .iter()
                    .filter(|p| p.simple_name() != Some("nil"))
                    .map(|p| self.strip_nil_resolved(p, guard))
                    .collect();
                match kept.len() {
                    0 => TypeExpr::named("nil"),
                    1 => kept.into_iter().next().unwrap(),
                    _ => TypeExpr::Union(kept),
                }
            }
            TypeExpr::Named(segs) if segs.len() == 1 => {
                let seg = &segs[0];
                let name = seg.name.as_str();
                if let Some(alias) = self.aliases.get(name) {
                    if guard.seen.insert(name.to_string()) {
                        let body = instantiate(alias, seg.args.as_deref());
                        let stripped = self.strip_nil_resolved(&body, guard);
                        guard.seen.remove(name);
                        if stripped == body {
                            arg.clone()
                        } else {
                            stripped
                        }
                    } else {
                        arg.clone()
                    }
                } else if let Some(expanded) = self.type_function(seg, guard) {
                    self.strip_nil_resolved(&expanded, guard)
                } else {
                    arg.clone()
                }
            }
            other => other.clone(),
        };
        guard.depth -= 1;
        result
    }

    fn resolve_dispatch(&self, ty: &TypeExpr, guard: &mut Guard) -> Resolved {
        match ty {
            TypeExpr::Named(segs) if segs.len() == 1 => {
                let seg = &segs[0];
                let name = seg.name.as_str();
                if !self.aliases.contains_key(name) {
                    if let Some(expanded) = self.type_function(seg, guard) {
                        return self.resolve_inner(&expanded, guard);
                    }
                }
                if let Some(alias) = self.aliases.get(name) {
                    if !guard.seen.insert(name.to_string()) {
                        return Resolved::Unresolved(name.to_string());
                    }
                    let body = instantiate(alias, seg.args.as_deref());
                    let r = self.resolve_inner(&body, guard);
                    guard.seen.remove(name);
                    return r;
                }
                if self.classes.contains(name) {
                    return Resolved::Class(name.to_string());
                }
                if self.enums.contains(name) {
                    return Resolved::Enum(name.to_string());
                }
                if self.interfaces.contains(name) {
                    return Resolved::Interface(name.to_string());
                }
                if seg.args.is_none() {
                    if let Some(t) = basic_value_type(name) {
                        return Resolved::Basic(t);
                    }
                }
                Resolved::Unresolved(name.to_string())
            }
            TypeExpr::Named(segs) if segs.len() == 2 => {
                match self.modules.get(&segs[0].name) {
                    Some(menv) => {
                        let inner = TypeExpr::Named(vec![segs[1].clone()]);
                        match menv.resolve_exported(&segs[1]) {
                            Some(r) => r,
                            None => Resolved::Unresolved(inner.to_string()),
                        }
                    }
                    None => Resolved::Structural(ty.clone()),
                }
            }
            TypeExpr::Named(_) => Resolved::Structural(ty.clone()),
            TypeExpr::StringLit(s) => Resolved::StringLiteral(s.clone()),
            TypeExpr::NumberLit(n) => Resolved::NumberLiteral(n.clone()),
            other => Resolved::Structural(other.clone()),
        }
    }

    fn resolve_exported(&self, seg: &NameSeg) -> Option<Resolved> {
        let name = seg.name.as_str();
        if let Some(alias) = self.aliases.get(name) {
            if alias.exported {
                let body = instantiate(alias, seg.args.as_deref());
                return Some(self.resolve_inner(&body, &mut Guard::new()));
            }
        }
        if self.classes.contains(name) {
            return Some(Resolved::Class(name.to_string()));
        }
        if self.enums.contains(name) {
            return Some(Resolved::Enum(name.to_string()));
        }
        None
    }

    pub fn value_type(&self, ty: &TypeExpr) -> Type {
        self.value_type_inner(ty, &mut Guard::new())
    }

    fn value_type_inner(&self, ty: &TypeExpr, guard: &mut Guard) -> Type {
        guard.depth += 1;
        if guard.too_deep() {
            guard.depth -= 1;
            return Type::Unknown;
        }
        let result = self.value_type_dispatch(ty, guard);
        guard.depth -= 1;
        result
    }

    fn value_type_dispatch(&self, ty: &TypeExpr, guard: &mut Guard) -> Type {
        match ty {
            TypeExpr::Named(segs) if segs.len() == 1 => {
                let seg = &segs[0];
                let name = seg.name.as_str();
                if !self.aliases.contains_key(name) {
                    if let Some(expanded) = self.type_function(seg, guard) {
                        return self.value_type_inner(&expanded, guard);
                    }
                }
                if let Some(alias) = self.aliases.get(name) {
                    if !guard.seen.insert(name.to_string()) {
                        return Type::Table(TableType {
                            fields: Vec::new(),
                            array: None,
                            name: Some(name.to_string()),
                        });
                    }
                    let body = instantiate(alias, seg.args.as_deref());
                    let mut r = self.value_type_inner(&body, guard);
                    guard.seen.remove(name);
                    if alias.generics.is_empty() {
                        if let Type::Table(tt) = &mut r {
                            if tt.name.is_none() {
                                tt.name = Some(name.to_string());
                            }
                        }
                    }
                    return r;
                }
                if self.classes.contains(name) {
                    return Type::Instance(name.to_string());
                }
                if self.enums.contains(name) {
                    return Type::EnumValue(name.to_string());
                }
                if self.interfaces.contains(name) {
                    return Type::Interface(name.to_string());
                }
                basic_value_type(name).unwrap_or(Type::Unknown)
            }
            TypeExpr::Named(segs) if segs.len() == 2 => {
                match self.modules.get(&segs[0].name) {
                    Some(menv) => menv.value_type_exported(&segs[1]),
                    None => Type::Unknown,
                }
            }
            TypeExpr::Named(_) => Type::Unknown,
            TypeExpr::StringLit(s) => Type::StringLit(s.clone()),
            TypeExpr::NumberLit(_) => Type::Number,
            TypeExpr::Optional(inner) => {
                Type::union_of(vec![self.value_type_inner(inner, guard), Type::Nil])
            }
            TypeExpr::Union(parts) => Type::union_of(
                parts
                    .iter()
                    .map(|p| self.value_type_inner(p, guard))
                    .collect(),
            ),
            TypeExpr::Intersection(parts) => {
                let mut fields: Vec<(String, Type)> = Vec::new();
                let mut array: Option<Box<Type>> = None;
                for p in parts {
                    match self.value_type_inner(p, guard) {
                        Type::Table(tt) => {
                            for (name, ty) in tt.fields {
                                match fields.iter_mut().find(|(n, _)| *n == name) {
                                    Some(slot) => slot.1 = ty,
                                    None => fields.push((name, ty)),
                                }
                            }
                            if tt.array.is_some() {
                                array = tt.array;
                            }
                        }
                        _ => return Type::Unknown,
                    }
                }
                Type::Table(TableType { fields, array, name: None })
            }
            TypeExpr::Table(t) => match t {
                TableTypeExpr::Empty | TableTypeExpr::Indexer { .. } => {
                    Type::Table(TableType::default())
                }
                TableTypeExpr::Record(fields) => Type::Table(TableType {
                    fields: fields
                        .iter()
                        .map(|f| {
                            let ft = self.value_type_inner(&f.ty, guard);
                            let ft = if f.optional {
                                Type::union_of(vec![ft, Type::Nil])
                            } else {
                                ft
                            };
                            (f.name.clone(), ft)
                        })
                        .collect(),
                    array: None,
                    name: None,
                }),
                TableTypeExpr::Array(elem) => Type::Table(TableType {
                    fields: Vec::new(),
                    array: Some(Box::new(self.value_type_inner(elem, guard))),
                    name: None,
                }),
            },
            TypeExpr::Function { params, ret } => {
                let mut infos = Vec::new();
                let mut is_vararg = false;
                for p in params {
                    match p {
                        Param::Positional { name, ty } => infos.push(ParamInfo {
                            name: name.clone().unwrap_or_default(),
                            ty: self.value_type_inner(ty, guard),
                        }),
                        Param::Vararg { .. } => is_vararg = true,
                    }
                }
                let returns = match ret.as_ref() {
                    TypeExpr::Tuple(parts) => parts
                        .iter()
                        .map(|p| self.value_type_inner(p, guard))
                        .collect(),
                    TypeExpr::Named(segs)
                        if segs.len() == 1
                            && segs[0].args.is_none()
                            && (segs[0].name == "nil" || segs[0].name == "void") =>
                    {
                        vec![Type::Nil]
                    }
                    other => vec![self.value_type_inner(other, guard)],
                };
                Type::Function(Some(Box::new(FunctionType {
                    params: infos,
                    is_vararg,
                    returns,
                    returns_param: None,
                    generic_sig: None,
                })))
            }
            TypeExpr::Tuple(parts) => match parts.len() {
                0 => Type::Nil,
                1 => self.value_type_inner(&parts[0], guard),
                _ => Type::Unknown,
            },
        }
    }

    fn value_type_exported(&self, seg: &NameSeg) -> Type {
        let name = seg.name.as_str();
        if let Some(alias) = self.aliases.get(name) {
            if alias.exported {
                let body = instantiate(alias, seg.args.as_deref());
                return self.value_type_inner(&body, &mut Guard::new());
            }
        }
        if self.classes.contains(name) {
            return Type::Instance(name.to_string());
        }
        if self.enums.contains(name) {
            return Type::EnumValue(name.to_string());
        }
        Type::Unknown
    }

    fn collect_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::TypeAlias { name, ty } => {
                    self.aliases.insert(
                        name.clone(),
                        TypeAlias {
                            exported: false,
                            name: name.clone(),
                            generics: Vec::new(),
                            ty: from_luar_type(ty),
                        },
                    );
                }
                Stmt::Class { name, members, .. } => {
                    self.classes.insert(name.clone());
                    for m in members {
                        match m {
                            ClassMember::Field { default, .. } => {
                                if let Some(e) = default {
                                    self.collect_expr(e);
                                }
                            }
                            ClassMember::Method { func, .. }
                            | ClassMember::Getter { func, .. }
                            | ClassMember::Setter { func, .. }
                            | ClassMember::Constructor { func }
                            | ClassMember::Destructor { func }
                            | ClassMember::Operator { func, .. } => {
                                self.collect_stmts(&func.body);
                            }
                        }
                    }
                }
                Stmt::Enum { name, .. } => {
                    self.enums.insert(name.clone());
                }
                Stmt::Interface { name, .. } => {
                    self.interfaces.insert(name.clone());
                }
                Stmt::Declare { inits, .. } => {
                    for e in inits {
                        self.collect_expr(e);
                    }
                }
                Stmt::Assign { values, .. } => {
                    for e in values {
                        self.collect_expr(e);
                    }
                }
                Stmt::Do(body) => self.collect_stmts(body),
                Stmt::If {
                    branches,
                    else_block,
                    ..
                } => {
                    for (cond, body) in branches {
                        self.collect_expr(cond);
                        self.collect_stmts(body);
                    }
                    if let Some(body) = else_block {
                        self.collect_stmts(body);
                    }
                }
                Stmt::While { cond, body, .. } => {
                    self.collect_expr(cond);
                    self.collect_stmts(body);
                }
                Stmt::ForNumeric {
                    start,
                    stop,
                    step,
                    body,
                    ..
                } => {
                    self.collect_expr(start);
                    self.collect_expr(stop);
                    if let Some(e) = step {
                        self.collect_expr(e);
                    }
                    self.collect_stmts(body);
                }
                Stmt::ForIn { iters, body, .. } => {
                    for e in iters {
                        self.collect_expr(e);
                    }
                    self.collect_stmts(body);
                }
                Stmt::Return { values, .. } => {
                    for e in values {
                        self.collect_expr(e);
                    }
                }
                Stmt::Buff { init, .. } => self.collect_expr(init),
                Stmt::Expr(e, _) => self.collect_expr(e),
                Stmt::Break { .. } | Stmt::FreeBuff { .. } => {}
            }
        }
    }

    fn collect_expr(&mut self, e: &Expr) {
        match e {
            Expr::Function { body, .. } => self.collect_stmts(body),
            Expr::Switch {
                subject,
                cases,
                default,
            } => {
                self.collect_expr(subject);
                for case in cases {
                    self.collect_expr(&case.pattern);
                    self.collect_stmts(&case.body);
                }
                if let Some(body) = default {
                    self.collect_stmts(body);
                }
            }
            Expr::Table(entries) => {
                for entry in entries {
                    match entry {
                        TableEntry::Positional(v) => self.collect_expr(v),
                        TableEntry::Keyed { key, value } => {
                            self.collect_expr(key);
                            self.collect_expr(value);
                        }
                    }
                }
            }
            Expr::Index { base, key } => {
                self.collect_expr(base);
                self.collect_expr(key);
            }
            Expr::Call { callee, args } => {
                self.collect_expr(callee);
                for a in args {
                    self.collect_expr(a);
                }
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.collect_expr(receiver);
                for a in args {
                    self.collect_expr(a);
                }
            }
            Expr::Unary { expr, .. } => self.collect_expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.collect_expr(lhs);
                self.collect_expr(rhs);
            }
            Expr::Logical { lhs, rhs, .. } => {
                self.collect_expr(lhs);
                self.collect_expr(rhs);
            }
            _ => {}
        }
    }
}

pub fn strip_nil_expr(arg: &TypeExpr) -> TypeExpr {
    match arg {
        TypeExpr::Optional(inner) => strip_nil_expr(inner),
        TypeExpr::Union(parts) => {
            let kept: Vec<TypeExpr> = parts
                .iter()
                .filter(|p| p.simple_name() != Some("nil"))
                .map(strip_nil_expr)
                .collect();
            match kept.len() {
                0 => TypeExpr::named("nil"),
                1 => kept.into_iter().next().unwrap(),
                _ => TypeExpr::Union(kept),
            }
        }
        other => other.clone(),
    }
}

fn basic_expr_of(value: &Type) -> Option<TypeExpr> {
    let name = match value {
        Type::Class(_) | Type::Instance(_) => "class",
        Type::Enum(_) | Type::EnumValue(_) => "enum",
        Type::Interface(_) => "interface",
        Type::Table(_) => "table",
        Type::Function(_) => "function",
        Type::Number => "number",
        Type::String | Type::StringLit(_) => "string",
        Type::Boolean => "boolean",
        Type::Nil => "nil",
        Type::Thread => "thread",
        Type::Union(parts) => {
            let mut out: Vec<TypeExpr> = Vec::new();
            for p in parts {
                let b = basic_expr_of(p)?;
                if !out.contains(&b) {
                    out.push(b);
                }
            }
            return match out.len() {
                0 => None,
                1 => Some(out.remove(0)),
                _ => Some(TypeExpr::Union(out)),
            };
        }
        Type::Unknown => return None,
    };
    Some(TypeExpr::named(name))
}

fn instantiate(alias: &TypeAlias, args: Option<&[TypeExpr]>) -> TypeExpr {
    match args {
        Some(args) if !alias.generics.is_empty() => {
            let map: HashMap<&str, &TypeExpr> = alias
                .generics
                .iter()
                .map(|g| g.as_str())
                .zip(args.iter())
                .collect();
            substitute(&alias.ty, &map, 0)
        }
        _ => alias.ty.clone(),
    }
}

pub(crate) fn substitute(ty: &TypeExpr, map: &HashMap<&str, &TypeExpr>, depth: usize) -> TypeExpr {
    if depth > MAX_RESOLVE_DEPTH {
        return ty.clone();
    }
    match ty {
        TypeExpr::Named(segs) if segs.len() == 1 && segs[0].args.is_none() => {
            match map.get(segs[0].name.as_str()) {
                Some(replacement) => (*replacement).clone(),
                None => ty.clone(),
            }
        }
        TypeExpr::Named(segs) => TypeExpr::Named(
            segs.iter()
                .map(|seg| NameSeg {
                    name: seg.name.clone(),
                    args: seg.args.as_ref().map(|args| {
                        args.iter().map(|a| substitute(a, map, depth + 1)).collect()
                    }),
                })
                .collect(),
        ),
        TypeExpr::StringLit(_) | TypeExpr::NumberLit(_) => ty.clone(),
        TypeExpr::Optional(inner) => {
            TypeExpr::Optional(Box::new(substitute(inner, map, depth + 1)))
        }
        TypeExpr::Union(parts) => {
            TypeExpr::Union(parts.iter().map(|p| substitute(p, map, depth + 1)).collect())
        }
        TypeExpr::Intersection(parts) => TypeExpr::Intersection(
            parts.iter().map(|p| substitute(p, map, depth + 1)).collect(),
        ),
        TypeExpr::Table(t) => TypeExpr::Table(match t {
            TableTypeExpr::Empty => TableTypeExpr::Empty,
            TableTypeExpr::Record(fields) => TableTypeExpr::Record(
                fields
                    .iter()
                    .map(|f| TableField {
                        name: f.name.clone(),
                        optional: f.optional,
                        ty: substitute(&f.ty, map, depth + 1),
                    })
                    .collect(),
            ),
            TableTypeExpr::Indexer { key, value } => TableTypeExpr::Indexer {
                key: Box::new(substitute(key, map, depth + 1)),
                value: Box::new(substitute(value, map, depth + 1)),
            },
            TableTypeExpr::Array(elem) => {
                TableTypeExpr::Array(Box::new(substitute(elem, map, depth + 1)))
            }
        }),
        TypeExpr::Function { params, ret } => TypeExpr::Function {
            params: params
                .iter()
                .map(|p| match p {
                    Param::Positional { name, ty } => Param::Positional {
                        name: name.clone(),
                        ty: substitute(ty, map, depth + 1),
                    },
                    Param::Vararg { ty } => Param::Vararg {
                        ty: ty.as_ref().map(|t| substitute(t, map, depth + 1)),
                    },
                })
                .collect(),
            ret: Box::new(substitute(ret, map, depth + 1)),
        },
        TypeExpr::Tuple(parts) => {
            TypeExpr::Tuple(parts.iter().map(|p| substitute(p, map, depth + 1)).collect())
        }
    }
}
