use crate::annotations::AnnotationSet;
use crate::builtins;
use crate::resolve::TypeEnv;
use crate::type_syntax::TypeExpr;
use crate::types::{FunctionType, GenericSig, ParamInfo, TableType, Type};
use luar::ast::{
    AssignOp, BinOp, ClassMember, Expr, LValue, LogicalOp, Mutability, Stmt, TableEntry, UnaryOp,
    Visibility,
};
use std::collections::HashMap;

#[derive(Default)]
pub struct InferOptions<'a> {
    pub globals: Vec<(String, Type)>,
    pub annotations: Option<&'a AnnotationSet>,
    pub env: Option<&'a TypeEnv>,
    pub require: Option<&'a (dyn Fn(&str) -> Option<Type> + Sync)>,
    pub classes: HashMap<String, ClassInfo>,
    pub enums: HashMap<String, EnumInfo>,
    pub ambient: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub name: String,
    pub line: Option<u32>,
    pub ty: Type,
    pub kind: BindingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Declare {
        visibility: Visibility,
        mutability: Mutability,
    },
    BareAssign,
    Assign,
    Class,
    Enum,
    Interface,
    Buff,
    LoopVar,
}

pub use luar::ast::Access;

#[derive(Debug, Clone, PartialEq)]
pub struct FieldInfo {
    pub name: String,
    pub is_static: bool,
    pub ty: Type,
    pub access: Access,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodInfo {
    pub name: String,
    pub is_static: bool,
    pub sig: FunctionType,
    pub access: Access,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropInfo {
    pub name: String,
    pub ty: Type,
    pub access: Access,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassInfo {
    pub name: String,
    pub is_pub: bool,
    pub parent: Option<String>,
    pub mixins: Vec<String>,
    pub interfaces: Vec<String>,
    pub is_abstract: bool,
    pub is_final: bool,
    pub fields: Vec<FieldInfo>,
    pub methods: Vec<MethodInfo>,
    pub getters: Vec<PropInfo>,
    pub setters: Vec<(String, Access)>,
    pub operators: Vec<(String, FunctionType)>,
    pub constructor: Option<FunctionType>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EnumInfo {
    pub name: String,
    pub is_pub: bool,
    pub variants: Vec<(String, Type)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Analysis {
    pub bindings: Vec<Binding>,
    pub classes: HashMap<String, ClassInfo>,
    pub enums: HashMap<String, EnumInfo>,
    pub interfaces: HashMap<String, Vec<String>>,
    pub aliases: Vec<(String, luar::ast::Type)>,
    pub module_returns: Vec<Type>,
}

impl Analysis {
    pub fn binding(&self, name: &str) -> Option<&Binding> {
        self.bindings.iter().rev().find(|b| b.name == name)
    }

    pub fn type_of(&self, name: &str) -> Option<&Type> {
        self.binding(name).map(|b| &b.ty)
    }
}

pub fn identify_program(stmts: &[Stmt]) -> Analysis {
    identify_program_with(stmts, &InferOptions::default())
}

pub fn identify_program_with(stmts: &[Stmt], opts: &InferOptions) -> Analysis {
    crate::scoped_large_stack(|| identify_program_unguarded(stmts, opts))
}

pub(crate) fn identify_program_unguarded(stmts: &[Stmt], opts: &InferOptions) -> Analysis {
    let mut inf = Inferencer::new(opts);
    inf.return_frames.push(Vec::new());
    inf.exec_block(stmts);
    let frame = inf.return_frames.pop().unwrap();
    inf.out.module_returns = merge_returns(frame);
    inf.out
}

pub fn identify_expr(expr: &Expr) -> Type {
    crate::scoped_large_stack(|| {
        let opts = InferOptions::default();
        let mut inf = Inferencer::new(&opts);
        inf.return_frames.push(Vec::new());
        inf.eval(expr)
    })
}

struct Inferencer<'a> {
    scopes: Vec<HashMap<String, Type>>,
    return_frames: Vec<Vec<Vec<Type>>>,
    out: Analysis,
    opts: &'a InferOptions<'a>,
    fallback_env: TypeEnv,
}

fn merge_returns(frame: Vec<Vec<Type>>) -> Vec<Type> {
    if frame.is_empty() {
        return Vec::new();
    }
    let max = frame.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(max);
    for pos in 0..max {
        let parts: Vec<Type> = frame
            .iter()
            .map(|r| r.get(pos).cloned().unwrap_or(Type::Nil))
            .collect();
        out.push(Type::union_of(parts));
    }
    out
}

fn arg_to_texpr(e: Option<&Expr>, t: Option<&Type>) -> Option<TypeExpr> {
    match e {
        Some(Expr::Str(s)) => Some(TypeExpr::StringLit(s.clone())),
        Some(Expr::Bool(_)) => Some(TypeExpr::named("boolean")),
        Some(Expr::Nil) => Some(TypeExpr::named("nil")),
        Some(Expr::Int(n)) => Some(TypeExpr::NumberLit(n.to_string())),
        Some(Expr::Float(x)) => Some(TypeExpr::NumberLit(x.to_string())),
        _ => t.and_then(type_to_texpr),
    }
}

fn type_to_texpr(t: &Type) -> Option<TypeExpr> {
    match t {
        Type::Number => Some(TypeExpr::named("number")),
        Type::String => Some(TypeExpr::named("string")),
        Type::StringLit(s) => Some(TypeExpr::StringLit(s.clone())),
        Type::Boolean => Some(TypeExpr::named("boolean")),
        Type::Nil => Some(TypeExpr::named("nil")),
        Type::Thread => Some(TypeExpr::named("thread")),
        Type::Instance(c) | Type::Class(c) => Some(TypeExpr::named(c)),
        Type::Enum(n) | Type::EnumValue(n) => Some(TypeExpr::named(n)),
        _ => None,
    }
}

fn metamethod_name(symbol: &str, arity: usize) -> Option<&'static str> {
    Some(match (symbol, arity) {
        ("+", _) => "__add",
        ("-", 0) => "__unm",
        ("-", _) => "__sub",
        ("*", _) => "__mul",
        ("/", _) => "__div",
        ("%", _) => "__mod",
        ("^", _) => "__pow",
        ("..", _) => "__concat",
        ("==", _) => "__eq",
        ("<", _) => "__lt",
        ("<=", _) => "__le",
        ("#", _) => "__len",
        _ => return None,
    })
}

fn op_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "^",
        BinOp::Concat => "..",
        BinOp::Eq => "==",
        BinOp::Ne => "~=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
    }
}

fn falsy_part(t: &Type) -> Option<Type> {
    match t {
        Type::Nil => Some(Type::Nil),
        Type::Boolean => Some(Type::Boolean),
        Type::Unknown => Some(Type::Unknown),
        Type::Union(parts) => {
            let fp: Vec<Type> = parts.iter().filter_map(falsy_part).collect();
            if fp.is_empty() {
                None
            } else {
                Some(Type::union_of(fp))
            }
        }
        _ => None,
    }
}

fn truthy_part(t: &Type) -> Option<Type> {
    match t {
        Type::Nil => None,
        Type::Unknown => Some(Type::Unknown),
        Type::Union(parts) => {
            let tp: Vec<Type> = parts.iter().filter_map(truthy_part).collect();
            if tp.is_empty() {
                None
            } else {
                Some(Type::union_of(tp))
            }
        }
        other => Some(other.clone()),
    }
}

impl<'a> Inferencer<'a> {
    fn new(opts: &'a InferOptions<'a>) -> Inferencer<'a> {
        let mut globals = builtins::global_env();
        for (name, ty) in &opts.globals {
            globals.insert(name.clone(), ty.clone());
        }
        Inferencer {
            scopes: vec![globals, HashMap::new()],
            return_frames: Vec::new(),
            out: Analysis::default(),
            opts,
            fallback_env: TypeEnv::default(),
        }
    }

    fn env(&self) -> &TypeEnv {
        self.opts.env.unwrap_or(&self.fallback_env)
    }

    fn annotated_var(&self, name: &str, line: u32) -> Option<Type> {
        let ann = self.opts.annotations?;
        let texpr = ann.var(name, line)?;
        Some(self.env().value_type(texpr))
    }

    fn bind(&mut self, name: &str, ty: Type, global: bool) {
        if global {
            self.scopes[0].insert(name.to_string(), ty);
        } else {
            self.scopes.last_mut().unwrap().insert(name.to_string(), ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<&Type> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn set_existing(&mut self, name: &str, ty: Type) -> bool {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(slot) = scope.get_mut(name) {
                *slot = ty;
                return true;
            }
        }
        false
    }

    fn exec_block(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.exec_stmt(s);
        }
    }

    fn exec_scoped(&mut self, stmts: &[Stmt]) {
        self.scopes.push(HashMap::new());
        self.exec_block(stmts);
        self.scopes.pop();
    }

    fn merge_scope_variants(&mut self, variants: Vec<Vec<HashMap<String, Type>>>) {
        if variants.is_empty() {
            return;
        }
        if variants.len() == 1 {
            self.scopes = variants.into_iter().next().unwrap();
            return;
        }
        let depth = variants.iter().map(|v| v.len()).min().unwrap_or(0);
        let mut merged: Vec<HashMap<String, Type>> = Vec::with_capacity(depth);
        for level in 0..depth {
            let mut keys: Vec<&String> = Vec::new();
            for v in &variants {
                for k in v[level].keys() {
                    if !keys.contains(&k) {
                        keys.push(k);
                    }
                }
            }
            let mut map = HashMap::new();
            for k in keys {
                let mut parts: Vec<Type> = Vec::new();
                for v in &variants {
                    if let Some(t) = v[level].get(k) {
                        if !parts.contains(t) {
                            parts.push(t.clone());
                        }
                    }
                }
                map.insert(k.clone(), Type::union_of(parts));
            }
            merged.push(map);
        }
        self.scopes = merged;
    }

    fn exec_branches(&mut self, bodies: &[&[Stmt]], include_fallthrough: bool) {
        if bodies.is_empty() {
            return;
        }
        if bodies.len() == 1 && !include_fallthrough {
            self.exec_scoped(bodies[0]);
            return;
        }
        let saved = self.scopes.clone();
        let mut variants: Vec<Vec<HashMap<String, Type>>> = Vec::new();
        if include_fallthrough {
            variants.push(saved.clone());
        }
        for body in bodies {
            self.scopes = saved.clone();
            self.exec_scoped(body);
            variants.push(std::mem::take(&mut self.scopes));
        }
        self.merge_scope_variants(variants);
    }

    fn exec_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Declare {
                visibility,
                mutability,
                names,
                inits,
                line,
            } => {
                if names.len() == 1 && inits.len() == 1 {
                    if let Expr::Function {
                        name: fname,
                        params,
                        is_vararg,
                        body,
                    } = &inits[0]
                    {
                        let is_pub = *visibility == Visibility::Pub;
                        self.bind(&names[0], Type::Function(None), is_pub);
                        let ptypes = self.fn_param_annotations(&names[0], *line);
                        let mut sig = self.infer_fn(
                            params,
                            *is_vararg,
                            body,
                            None,
                            Some(fname),
                            ptypes.as_ref(),
                        );
                        if let Some(ann) = self.opts.annotations {
                            let key = (names[0].clone(), *line);
                            if let Some(ret) = ann.fn_returns.get(&key) {
                                sig.returns = self.annotation_returns(ret);
                                if let Some(generics) = ann.fn_generics.get(&key) {
                                    let param_anns: Vec<Option<TypeExpr>> = params
                                        .iter()
                                        .map(|p| {
                                            ann.fn_params
                                                .get(&key)
                                                .and_then(|m| m.get(p).cloned())
                                        })
                                        .collect();
                                    if let Some(g) = ret.simple_name() {
                                        if generics.iter().any(|x| x == g) {
                                            sig.returns_param =
                                                param_anns.iter().position(|t| {
                                                    t.as_ref().and_then(|t| t.simple_name())
                                                        == Some(g)
                                                });
                                        }
                                    }
                                    sig.generic_sig = Some(Box::new(GenericSig {
                                        generics: generics.clone(),
                                        param_anns,
                                        ret_ann: ret.clone(),
                                    }));
                                }
                            }
                        }
                        let ty = Type::Function(Some(Box::new(sig)));
                        let ty = self.annotated_var(&names[0], *line).unwrap_or(ty);
                        self.bind(&names[0], ty.clone(), is_pub);
                        self.out.bindings.push(Binding {
                            name: names[0].clone(),
                            line: Some(*line),
                            ty,
                            kind: BindingKind::Declare {
                                visibility: *visibility,
                                mutability: *mutability,
                            },
                        });
                        return;
                    }
                }
                let values = self.eval_multi(inits, names.len());
                for (name, ty) in names.iter().zip(values) {
                    let ty = self.annotated_var(name, *line).unwrap_or(ty);
                    self.bind(name, ty.clone(), *visibility == Visibility::Pub);
                    self.out.bindings.push(Binding {
                        name: name.clone(),
                        line: Some(*line),
                        ty,
                        kind: BindingKind::Declare {
                            visibility: *visibility,
                            mutability: *mutability,
                        },
                    });
                }
            }
            Stmt::Assign {
                targets,
                op,
                values,
                line,
            } => {
                let vals = self.eval_multi(values, targets.len());
                for (i, (target, val)) in targets.iter().zip(vals).enumerate() {
                    match target {
                        LValue::Name(name) => {
                            let computed = if *op == AssignOp::Assign {
                                val
                            } else {
                                let cur = self.lookup(name).cloned().unwrap_or(Type::Unknown);
                                self.compound_result(*op, &cur)
                            };
                            let new_ty = self.annotated_var(name, *line).unwrap_or(computed);
                            let existed = self.set_existing(name, new_ty.clone());
                            if !existed {
                                self.bind(name, new_ty.clone(), false);
                            }
                            self.out.bindings.push(Binding {
                                name: name.clone(),
                                line: Some(*line),
                                ty: new_ty,
                                kind: if existed {
                                    BindingKind::Assign
                                } else {
                                    BindingKind::BareAssign
                                },
                            });
                        }
                        LValue::Index { base, key } => {
                            let mut new_val = if *op == AssignOp::Assign {
                                val
                            } else {
                                let cur = self.eval_index(base, key);
                                self.compound_result(*op, &cur)
                            };
                            if *op == AssignOp::Assign {
                                if let (
                                    Some(Expr::Function {
                                        name: fname,
                                        params,
                                        is_vararg,
                                        body,
                                    }),
                                    Expr::Str(k),
                                ) = (values.get(i), key.as_ref())
                                {
                                    new_val = self.annotated_function_type(
                                        k,
                                        *line,
                                        params,
                                        *is_vararg,
                                        body,
                                        fname,
                                    );
                                }
                            }
                            self.assign_index(base, key, new_val);
                        }
                    }
                }
            }
            Stmt::Do(body) => self.exec_scoped(body),
            Stmt::If {
                branches,
                else_block,
                ..
            } => {
                for (cond, _) in branches {
                    self.eval(cond);
                }
                let mut bodies: Vec<&[Stmt]> =
                    branches.iter().map(|(_, b)| b.as_slice()).collect();
                if let Some(eb) = else_block {
                    bodies.push(eb.as_slice());
                }
                self.exec_branches(&bodies, else_block.is_none());
            }
            Stmt::While { cond, body, .. } => {
                self.eval(cond);
                self.exec_branches(&[body.as_slice()], true);
            }
            Stmt::ForNumeric {
                var,
                start,
                stop,
                step,
                body,
                line,
            } => {
                self.eval(start);
                self.eval(stop);
                if let Some(s) = step {
                    self.eval(s);
                }
                let saved = self.scopes.clone();
                self.scopes.push(HashMap::new());
                self.bind(var, Type::Number, false);
                self.out.bindings.push(Binding {
                    name: var.clone(),
                    line: Some(*line),
                    ty: Type::Number,
                    kind: BindingKind::LoopVar,
                });
                self.exec_block(body);
                self.scopes.pop();
                let run = std::mem::replace(&mut self.scopes, saved.clone());
                self.merge_scope_variants(vec![saved, run]);
            }
            Stmt::ForIn { names, iters, body, line } => {
                let var_types = self.for_in_types(names, iters);
                let saved = self.scopes.clone();
                self.scopes.push(HashMap::new());
                for (name, ty) in names.iter().zip(var_types) {
                    self.bind(name, ty.clone(), false);
                    self.out.bindings.push(Binding {
                        name: name.clone(),
                        line: Some(*line),
                        ty,
                        kind: BindingKind::LoopVar,
                    });
                }
                self.exec_block(body);
                self.scopes.pop();
                let run = std::mem::replace(&mut self.scopes, saved.clone());
                self.merge_scope_variants(vec![saved, run]);
            }
            Stmt::Break { .. } => {}
            Stmt::Return { values, .. } => {
                let tys = self.eval_return_values(values);
                if let Some(frame) = self.return_frames.last_mut() {
                    frame.push(tys);
                }
            }
            Stmt::TypeAlias { name, ty } => {
                self.out.aliases.push((name.clone(), ty.clone()));
            }
            Stmt::Buff {
                name, init, line, ..
            } => {
                let ty = self.eval(init);
                let module_scope = if self.scopes.len() > 1 { 1 } else { 0 };
                self.scopes[module_scope].insert(name.clone(), ty.clone());
                self.out.bindings.push(Binding {
                    name: name.clone(),
                    line: Some(*line),
                    ty,
                    kind: BindingKind::Buff,
                });
            }
            Stmt::FreeBuff { name, .. } => {
                for scope in self.scopes.iter_mut() {
                    scope.remove(name);
                }
            }
            Stmt::Class {
                visibility,
                is_final,
                is_abstract,
                name,
                parent,
                mixins,
                interfaces,
                members,
            } => {
                self.exec_class(
                    *visibility,
                    *is_final,
                    *is_abstract,
                    name,
                    parent,
                    mixins,
                    interfaces,
                    members,
                );
            }
            Stmt::Interface {
                visibility,
                name,
                members,
                ..
            } => {
                self.out.interfaces.insert(name.clone(), members.clone());
                self.bind(
                    name,
                    Type::Interface(name.clone()),
                    *visibility == Visibility::Pub,
                );
                self.out.bindings.push(Binding {
                    name: name.clone(),
                    line: None,
                    ty: Type::Interface(name.clone()),
                    kind: BindingKind::Interface,
                });
            }
            Stmt::Enum {
                visibility,
                name,
                variants,
                line,
            } => {
                let mut variant_types = Vec::new();
                for (vname, value) in variants {
                    let ty = match value {
                        Some(e) => self.eval(e),
                        None => Type::Number,
                    };
                    variant_types.push((vname.clone(), ty));
                }
                let is_pub = *visibility == Visibility::Pub;
                let info = self
                    .out
                    .enums
                    .entry(name.clone())
                    .or_insert_with(|| EnumInfo {
                        name: name.clone(),
                        is_pub,
                        variants: Vec::new(),
                    });
                info.is_pub |= is_pub;
                for (vname, ty) in variant_types {
                    match info.variants.iter_mut().find(|(n, _)| n == &vname) {
                        Some(slot) => slot.1 = ty,
                        None => info.variants.push((vname, ty)),
                    }
                }
                self.bind(name, Type::Enum(name.clone()), true);
                self.out.bindings.push(Binding {
                    name: name.clone(),
                    line: Some(*line),
                    ty: Type::Enum(name.clone()),
                    kind: BindingKind::Enum,
                });
            }
            Stmt::Expr(e, _) => {
                self.eval(e);
            }
        }
    }

    fn annotation_returns(&self, ret: &TypeExpr) -> Vec<Type> {
        match ret {
            TypeExpr::Tuple(parts) => parts.iter().map(|p| self.env().value_type(p)).collect(),
            single => match single.simple_name() {
                Some("void") | Some("nil") => vec![Type::Nil],
                _ => vec![self.env().value_type(single)],
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_class(
        &mut self,
        visibility: Visibility,
        is_final: bool,
        is_abstract: bool,
        name: &str,
        parent: &Option<String>,
        mixins: &[String],
        interfaces: &[String],
        members: &[ClassMember],
    ) {
        let info = ClassInfo {
            name: name.to_string(),
            is_pub: visibility == Visibility::Pub,
            parent: parent.clone(),
            mixins: mixins.to_vec(),
            interfaces: interfaces.to_vec(),
            is_abstract,
            is_final,
            fields: Vec::new(),
            methods: Vec::new(),
            getters: Vec::new(),
            setters: Vec::new(),
            operators: Vec::new(),
            constructor: None,
        };
        self.out.classes.insert(name.to_string(), info);
        self.bind(
            name,
            Type::Class(name.to_string()),
            visibility == Visibility::Pub,
        );
        self.out.bindings.push(Binding {
            name: name.to_string(),
            line: None,
            ty: Type::Class(name.to_string()),
            kind: BindingKind::Class,
        });

        for member in members {
            if let ClassMember::Field {
                access,
                is_static,
                name: fname,
                default,
            } = member
            {
                let annotated = self.opts.annotations.and_then(|a| {
                    a.class_fields
                        .get(&(name.to_string(), fname.clone()))
                        .map(|t| self.env().value_type(t))
                });
                let ty = match (annotated, default) {
                    (Some(t), _) => t,
                    (None, Some(e)) => self.eval(e),
                    (None, None) => Type::Unknown,
                };
                if let Some(info) = self.out.classes.get_mut(name) {
                    info.fields.push(FieldInfo {
                        name: fname.clone(),
                        is_static: *is_static,
                        ty,
                        access: *access,
                    });
                }
            }
        }

        for member in members {
            match member {
                ClassMember::Field { .. } => {}
                ClassMember::Method {
                    access,
                    is_static,
                    name: mname,
                    func,
                    ..
                } => {
                    let self_class = if *is_static { None } else { Some(name) };
                    let params = self.method_param_annotations(name, mname);
                    let mut sig = self.infer_fn(
                        &func.params,
                        func.is_vararg,
                        &func.body,
                        self_class,
                        None,
                        params.as_ref(),
                    );
                    if let Some(ann) = self.opts.annotations {
                        if let Some(ret) =
                            ann.method_returns.get(&(name.to_string(), mname.clone()))
                        {
                            sig.returns = self.annotation_returns(ret);
                        }
                    }
                    if let Some(info) = self.out.classes.get_mut(name) {
                        info.methods.push(MethodInfo {
                            name: mname.clone(),
                            is_static: *is_static,
                            sig,
                            access: *access,
                        });
                    }
                }
                ClassMember::Getter {
                    access,
                    name: gname,
                    func,
                } => {
                    let sig = self.infer_fn(
                        &func.params,
                        func.is_vararg,
                        &func.body,
                        Some(name),
                        None,
                        None,
                    );
                    let annotated = self.opts.annotations.and_then(|a| {
                        a.getter_returns
                            .get(&(name.to_string(), gname.clone()))
                            .map(|t| self.env().value_type(t))
                    });
                    let ty = annotated
                        .unwrap_or_else(|| sig.returns.first().cloned().unwrap_or(Type::Nil));
                    if let Some(info) = self.out.classes.get_mut(name) {
                        info.getters.push(PropInfo {
                            name: gname.clone(),
                            ty,
                            access: *access,
                        });
                    }
                }
                ClassMember::Setter {
                    access,
                    name: sname,
                    func,
                } => {
                    self.infer_fn(
                        &func.params,
                        func.is_vararg,
                        &func.body,
                        Some(name),
                        None,
                        None,
                    );
                    if let Some(info) = self.out.classes.get_mut(name) {
                        info.setters.push((sname.clone(), *access));
                    }
                }
                ClassMember::Constructor { func } => {
                    let params = self.method_param_annotations(name, "constructor");
                    let sig = self.infer_fn(
                        &func.params,
                        func.is_vararg,
                        &func.body,
                        Some(name),
                        None,
                        params.as_ref(),
                    );
                    if let Some(info) = self.out.classes.get_mut(name) {
                        info.constructor = Some(sig);
                    }
                }
                ClassMember::Destructor { func } => {
                    self.infer_fn(
                        &func.params,
                        func.is_vararg,
                        &func.body,
                        Some(name),
                        None,
                        None,
                    );
                }
                ClassMember::Operator { symbol, func } => {
                    let sig = self.infer_fn(
                        &func.params,
                        func.is_vararg,
                        &func.body,
                        Some(name),
                        None,
                        None,
                    );
                    if let Some(info) = self.out.classes.get_mut(name) {
                        info.operators.push((symbol.clone(), sig));
                    }
                }
            }
        }
    }

    fn method_param_annotations(&self, class: &str, method: &str) -> Option<HashMap<String, Type>> {
        let ann = self.opts.annotations?;
        let params = ann
            .method_params
            .get(&(class.to_string(), method.to_string()))?;
        Some(
            params
                .iter()
                .map(|(k, v)| (k.clone(), self.env().value_type(v)))
                .collect(),
        )
    }

    fn fn_param_annotations(&self, name: &str, line: u32) -> Option<HashMap<String, Type>> {
        let ann = self.opts.annotations?;
        let params = ann.fn_params.get(&(name.to_string(), line))?;
        Some(
            params
                .iter()
                .map(|(k, v)| (k.clone(), self.env().value_type(v)))
                .collect(),
        )
    }

    fn annotated_function_type(
        &mut self,
        key_name: &str,
        line: u32,
        params: &[String],
        is_vararg: bool,
        body: &[Stmt],
        fname: &str,
    ) -> Type {
        let ptypes = self.fn_param_annotations(key_name, line);
        let mut sig = self.infer_fn(params, is_vararg, body, None, Some(fname), ptypes.as_ref());
        if let Some(ann) = self.opts.annotations {
            if let Some(ret) = ann.fn_returns.get(&(key_name.to_string(), line)) {
                sig.returns = self.annotation_returns(ret);
            }
        }
        Type::Function(Some(Box::new(sig)))
    }

    fn infer_fn(
        &mut self,
        params: &[String],
        is_vararg: bool,
        body: &[Stmt],
        self_class: Option<&str>,
        self_name: Option<&str>,
        param_types: Option<&HashMap<String, Type>>,
    ) -> FunctionType {
        self.scopes.push(HashMap::new());
        if let Some(class) = self_class {
            self.bind("self", Type::Instance(class.to_string()), false);
            let parent = self
                .out
                .classes
                .get(class)
                .and_then(|c| c.parent.clone());
            match parent {
                Some(p) => self.bind("super", Type::Instance(p), false),
                None => self.bind("super", Type::Unknown, false),
            }
        }
        if let Some(n) = self_name {
            if !n.is_empty() {
                self.bind(n, Type::Function(None), false);
            }
        }
        let mut param_infos = Vec::with_capacity(params.len());
        for p in params {
            let ty = param_types
                .and_then(|m| m.get(p).cloned())
                .unwrap_or(Type::Unknown);
            self.bind(p, ty.clone(), false);
            param_infos.push(ParamInfo {
                name: p.clone(),
                ty,
            });
        }
        self.return_frames.push(Vec::new());
        self.exec_block(body);
        let frame = self.return_frames.pop().unwrap();
        self.scopes.pop();
        FunctionType {
            params: param_infos,
            is_vararg,
            returns: merge_returns(frame),
            returns_param: None,
            generic_sig: None,
        }
    }

    fn eval_multi(&mut self, exprs: &[Expr], want: usize) -> Vec<Type> {
        let mut out = Vec::new();
        let mut open_ended = false;
        for (i, e) in exprs.iter().enumerate() {
            if i + 1 == exprs.len() && want > exprs.len() {
                let (vals, open) = self.eval_full(e);
                out.extend(vals);
                open_ended = open;
            } else {
                out.push(self.eval(e));
            }
        }
        while out.len() < want {
            out.push(if open_ended { Type::Unknown } else { Type::Nil });
        }
        out.truncate(want);
        out
    }

    fn eval_return_values(&mut self, values: &[Expr]) -> Vec<Type> {
        let mut out = Vec::new();
        for (i, e) in values.iter().enumerate() {
            if i + 1 == values.len() {
                let (vals, open) = self.eval_full(e);
                out.extend(vals);
                if open && out.len() == i {
                    out.push(Type::Unknown);
                }
            } else {
                out.push(self.eval(e));
            }
        }
        out
    }

    fn eval_full(&mut self, e: &Expr) -> (Vec<Type>, bool) {
        match e {
            Expr::Call { callee, args } => self.eval_call(callee, args),
            Expr::MethodCall {
                receiver,
                method,
                args,
            } => self.eval_method_call(receiver, method, args),
            Expr::Vararg => (vec![Type::Unknown], true),
            other => (vec![self.eval(other)], false),
        }
    }

    fn eval(&mut self, e: &Expr) -> Type {
        match e {
            Expr::Nil => Type::Nil,
            Expr::Bool(_) => Type::Boolean,
            Expr::Int(_) => Type::Number,
            Expr::Float(_) => Type::Number,
            Expr::Str(_) => Type::String,
            Expr::Vararg => Type::Unknown,
            Expr::Name(n) => self.lookup(n).cloned().unwrap_or(Type::Unknown),
            Expr::Table(entries) => self.eval_table(entries),
            Expr::Index { base, key } => self.eval_index(base, key),
            Expr::Call { callee, args } => {
                let (vals, open) = self.eval_call(callee, args);
                vals.into_iter().next().unwrap_or(if open {
                    Type::Unknown
                } else {
                    Type::Nil
                })
            }
            Expr::MethodCall {
                receiver,
                method,
                args,
            } => {
                let (vals, open) = self.eval_method_call(receiver, method, args);
                vals.into_iter().next().unwrap_or(if open {
                    Type::Unknown
                } else {
                    Type::Nil
                })
            }
            Expr::Function {
                name,
                params,
                is_vararg,
                body,
            } => {
                let sig = self.infer_fn(params, *is_vararg, body, None, Some(name), None);
                Type::Function(Some(Box::new(sig)))
            }
            Expr::Switch {
                subject,
                cases,
                default,
            } => {
                self.eval(subject);
                for case in cases {
                    self.eval(&case.pattern);
                }
                let saved = self.scopes.clone();
                let mut variants: Vec<Vec<HashMap<String, Type>>> = Vec::new();
                if default.is_none() {
                    variants.push(saved.clone());
                }
                let mut parts = Vec::new();
                for case in cases {
                    self.scopes = saved.clone();
                    parts.push(self.eval_switch_body(&case.body));
                    variants.push(std::mem::take(&mut self.scopes));
                }
                match default {
                    Some(body) => {
                        self.scopes = saved.clone();
                        parts.push(self.eval_switch_body(body));
                        variants.push(std::mem::take(&mut self.scopes));
                    }
                    None => parts.push(Type::Nil),
                }
                self.merge_scope_variants(variants);
                Type::union_of(parts)
            }
            Expr::Unary { op, expr } => {
                let t = self.eval(expr);
                match op {
                    UnaryOp::Not => Type::Boolean,
                    UnaryOp::Len => self.overload_or(&t, &Type::Nil, "#", 0, Type::Number),
                    UnaryOp::Neg => self.overload_or(&t, &Type::Nil, "-", 0, Type::Number),
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                let lt = self.eval(lhs);
                let rt = self.eval(rhs);
                match op {
                    BinOp::Ne => Type::Boolean,
                    BinOp::Eq => self.overload_or(&lt, &rt, "==", 1, Type::Boolean),
                    BinOp::Lt | BinOp::Gt => {
                        self.overload_or(&lt, &rt, "<", 1, Type::Boolean)
                    }
                    BinOp::Le | BinOp::Ge => {
                        self.overload_or(&lt, &rt, "<=", 1, Type::Boolean)
                    }
                    BinOp::Concat => self.overload_or(&lt, &rt, "..", 1, Type::String),
                    other => self.overload_or(&lt, &rt, op_symbol(*other), 1, Type::Number),
                }
            }
            Expr::Logical { op, lhs, rhs } => {
                let lt = self.eval(lhs);
                let rt = self.eval(rhs);
                match op {
                    LogicalOp::And => match falsy_part(&lt) {
                        None => rt,
                        Some(fp) => {
                            if truthy_part(&lt).is_none() {
                                fp
                            } else {
                                Type::union_of(vec![fp, rt])
                            }
                        }
                    },
                    LogicalOp::Or => match truthy_part(&lt) {
                        None => rt,
                        Some(tp) => {
                            if falsy_part(&lt).is_none() {
                                tp
                            } else {
                                Type::union_of(vec![tp, rt])
                            }
                        }
                    },
                }
            }
        }
    }

    fn eval_switch_body(&mut self, body: &[Stmt]) -> Type {
        self.scopes.push(HashMap::new());
        self.return_frames.push(Vec::new());
        self.exec_block(body);
        let frame = self.return_frames.pop().unwrap();
        self.scopes.pop();
        if frame.is_empty() {
            Type::Nil
        } else {
            merge_returns(frame).into_iter().next().unwrap_or(Type::Nil)
        }
    }

    fn eval_table(&mut self, entries: &[TableEntry]) -> Type {
        let mut fields: Vec<(String, Type)> = Vec::new();
        let mut elems: Vec<Type> = Vec::new();
        let last_positional = entries
            .iter()
            .rposition(|e| matches!(e, TableEntry::Positional(_)));
        for (idx, entry) in entries.iter().enumerate() {
            match entry {
                TableEntry::Positional(e) => {
                    if Some(idx) == last_positional && idx + 1 == entries.len() {
                        let (vals, _) = self.eval_full(e);
                        elems.extend(vals);
                    } else {
                        elems.push(self.eval(e));
                    }
                }
                TableEntry::Keyed { key, value } => {
                    let vty = self.eval(value);
                    match key {
                        Expr::Str(s) => fields.push((s.clone(), vty)),
                        Expr::Int(_) | Expr::Float(_) => elems.push(vty),
                        other => {
                            self.eval(other);
                        }
                    }
                }
            }
        }
        let array = if elems.is_empty() {
            None
        } else {
            Some(Box::new(Type::union_of(elems)))
        };
        Type::Table(TableType { fields, array, name: None })
    }

    fn eval_index(&mut self, base: &Expr, key: &Expr) -> Type {
        let bty = self.eval(base);
        let key_name = match key {
            Expr::Str(s) => Some(s.as_str()),
            _ => None,
        };
        match &bty {
            Type::Enum(name) => match key_name {
                Some(k) => {
                    let known = self
                        .out
                        .enums
                        .get(name)
                        .or_else(|| self.opts.enums.get(name))
                        .map(|e| e.variants.iter().any(|(n, _)| n == k))
                        .unwrap_or(false);
                    if known {
                        Type::EnumValue(name.clone())
                    } else {
                        Type::Unknown
                    }
                }
                None => Type::EnumValue(name.clone()),
            },
            Type::Instance(class) => match key_name {
                Some(k) => self.instance_member_type(class, k),
                None => Type::Unknown,
            },
            Type::Class(class) => match key_name {
                Some(k) => self.static_member_type(class, k),
                None => Type::Unknown,
            },
            Type::Table(tt) => match key_name {
                Some(k) => self.table_field(tt, k).unwrap_or(Type::Unknown),
                None => match (&tt.array, key) {
                    (Some(elem), Expr::Int(_) | Expr::Float(_)) => (**elem).clone(),
                    (Some(elem), _) => {
                        let kt = self.eval(key);
                        if kt == Type::Number {
                            (**elem).clone()
                        } else {
                            Type::Unknown
                        }
                    }
                    (None, _) => {
                        self.eval(key);
                        Type::Unknown
                    }
                },
            },
            _ => {
                self.eval(key);
                Type::Unknown
            }
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr]) -> (Vec<Type>, bool) {
        let arg_types: Vec<Type> = args.iter().map(|a| self.eval(a)).collect();
        if let Expr::Name(n) = callee {
            match n.as_str() {
                "require" if !args.is_empty() => {
                    if let (Some(hook), Expr::Str(path)) = (self.opts.require, &args[0]) {
                        if let Some(ty) = hook(path) {
                            return (vec![ty], false);
                        }
                    }
                }
                "pcall" if !args.is_empty() => {
                    if let Some(Type::Function(Some(ft))) = arg_types.first() {
                        let mut rets = vec![Type::Boolean];
                        rets.extend(ft.returns.iter().cloned());
                        return (rets, false);
                    }
                    return (vec![Type::Boolean], true);
                }
                "assert" if !args.is_empty() => {
                    return (arg_types.clone(), false);
                }
                _ => {}
            }
        }
        let cty = self.eval(callee);
        match cty {
            Type::Class(c) => (vec![Type::Instance(c)], false),
            Type::Function(Some(ft)) => {
                if let Some(i) = ft.returns_param {
                    if let Some(t) = arg_types.get(i) {
                        return (vec![t.clone()], false);
                    }
                }
                if let Some(gs) = &ft.generic_sig {
                    if let Some(ret) = self.instantiate_generic_call(gs, args, &arg_types) {
                        return (vec![ret], false);
                    }
                }
                (ft.returns.clone(), false)
            }
            _ => (vec![], true),
        }
    }

    fn instantiate_generic_call(
        &self,
        gs: &GenericSig,
        args: &[Expr],
        arg_types: &[Type],
    ) -> Option<Type> {
        let mut bound: Vec<(String, TypeExpr)> = Vec::new();
        for g in &gs.generics {
            let from_param = gs.param_anns.iter().enumerate().find_map(|(i, pann)| {
                if pann.as_ref().and_then(|t| t.simple_name()) == Some(g.as_str()) {
                    arg_to_texpr(args.get(i), arg_types.get(i))
                } else {
                    None
                }
            });
            let value = from_param.or_else(|| arg_to_texpr(args.first(), arg_types.first()))?;
            bound.push((g.clone(), value));
        }
        let map: HashMap<&str, &TypeExpr> = bound
            .iter()
            .map(|(g, t)| (g.as_str(), t))
            .collect();
        let substituted = crate::resolve::substitute(&gs.ret_ann, &map, 0);
        let result = self.env().value_type(&substituted);
        if result == Type::Unknown {
            None
        } else {
            Some(result)
        }
    }

    fn eval_method_call(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> (Vec<Type>, bool) {
        for a in args {
            self.eval(a);
        }
        let recv = self.eval(receiver);
        match &recv {
            Type::Instance(c) | Type::Class(c) => match self.find_method(c, method) {
                Some(sig) => (sig.returns.clone(), false),
                None => (vec![], true),
            },
            Type::Table(tt) => match self.table_field(tt, method) {
                Some(Type::Function(Some(ft))) => (ft.returns.clone(), false),
                _ => (vec![], true),
            },
            _ => (vec![], true),
        }
    }

    fn compound_result(&mut self, op: AssignOp, cur: &Type) -> Type {
        match op {
            AssignOp::Assign => cur.clone(),
            AssignOp::Concat => self.overload_or(cur, &Type::Nil, "..", 1, Type::String),
            AssignOp::Add => self.overload_or(cur, &Type::Nil, "+", 1, Type::Number),
            AssignOp::Sub => self.overload_or(cur, &Type::Nil, "-", 1, Type::Number),
            AssignOp::Mul => self.overload_or(cur, &Type::Nil, "*", 1, Type::Number),
            AssignOp::Div => self.overload_or(cur, &Type::Nil, "/", 1, Type::Number),
            AssignOp::Mod => self.overload_or(cur, &Type::Nil, "%", 1, Type::Number),
        }
    }

    fn overload_or(
        &self,
        lt: &Type,
        rt: &Type,
        symbol: &str,
        arity: usize,
        fallback: Type,
    ) -> Type {
        for t in [lt, rt] {
            match t {
                Type::Instance(c) => {
                    if let Some(sig) = self.find_operator(c, symbol, arity) {
                        return sig.returns.first().cloned().unwrap_or(Type::Unknown);
                    }
                }
                Type::Table(tt) => {
                    if let Some(meta) = metamethod_name(symbol, arity) {
                        if let Some(Type::Function(Some(ft))) = self.table_field(tt, meta) {
                            return ft.returns.first().cloned().unwrap_or(Type::Unknown);
                        }
                    }
                }
                _ => {}
            }
        }
        fallback
    }

    fn expand_named_table(&self, tt: &TableType) -> Option<TableType> {
        if !tt.fields.is_empty() || tt.array.is_some() {
            return None;
        }
        let name = tt.name.as_ref()?;
        if name == "table" {
            return None;
        }
        match self.env().value_type(&TypeExpr::named(name)) {
            Type::Table(out) if !out.fields.is_empty() || out.array.is_some() => Some(out),
            _ => None,
        }
    }

    fn table_field(&self, tt: &TableType, name: &str) -> Option<Type> {
        if let Some((_, t)) = tt.fields.iter().find(|(n, _)| n == name) {
            return Some(t.clone());
        }
        let expanded = self.expand_named_table(tt)?;
        expanded
            .fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.clone())
    }

    fn walk_class_chain<'b, F, R>(&'b self, class: &str, include_mixins: bool, f: &F) -> Option<R>
    where
        F: Fn(&'b ClassInfo) -> Option<R>,
    {
        let mut current = Some(class.to_string());
        let mut guard = 0;
        while let Some(cname) = current {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let info = self
                .out
                .classes
                .get(&cname)
                .or_else(|| self.opts.classes.get(&cname))?;
            if let Some(r) = f(info) {
                return Some(r);
            }
            if include_mixins {
                for mixin in info.mixins.iter().rev() {
                    if let Some(mi) = self
                        .out
                        .classes
                        .get(mixin)
                        .or_else(|| self.opts.classes.get(mixin))
                    {
                        if let Some(r) = f(mi) {
                            return Some(r);
                        }
                    }
                }
            }
            current = info.parent.clone();
        }
        None
    }

    fn find_method(&self, class: &str, name: &str) -> Option<FunctionType> {
        self.walk_class_chain(class, true, &|info: &ClassInfo| {
            info.methods
                .iter()
                .find(|m| m.name == name)
                .map(|m| m.sig.clone())
        })
    }

    fn find_operator(&self, class: &str, symbol: &str, arity: usize) -> Option<FunctionType> {
        self.walk_class_chain(class, true, &|info: &ClassInfo| {
            info.operators
                .iter()
                .find(|(s, sig)| s == symbol && sig.params.len() == arity)
                .map(|(_, sig)| sig.clone())
        })
    }

    fn instance_member_type(&self, class: &str, name: &str) -> Type {
        if let Some(ty) = self.walk_class_chain(class, false, &|info: &ClassInfo| {
            info.getters
                .iter()
                .find(|g| g.name == name)
                .map(|g| g.ty.clone())
        }) {
            return ty;
        }
        if let Some(ty) = self.walk_class_chain(class, false, &|info: &ClassInfo| {
            info.fields
                .iter()
                .find(|f| f.name == name && !f.is_static)
                .map(|f| f.ty.clone())
        }) {
            return ty;
        }
        if let Some(ty) = self.walk_class_chain(class, true, &|info: &ClassInfo| {
            info.fields
                .iter()
                .find(|f| f.name == name && !f.is_static)
                .map(|f| f.ty.clone())
        }) {
            return ty;
        }
        Type::Unknown
    }

    fn static_member_type(&self, class: &str, name: &str) -> Type {
        self.walk_class_chain(class, false, &|info: &ClassInfo| {
            if let Some(field) = info.fields.iter().find(|f| f.name == name && f.is_static) {
                return Some(field.ty.clone());
            }
            info.methods
                .iter()
                .find(|m| m.name == name && m.is_static)
                .map(|m| Type::Function(Some(Box::new(m.sig.clone()))))
        })
        .unwrap_or(Type::Unknown)
    }

    fn assign_index(&mut self, base: &Expr, key: &Expr, val: Type) {
        let key_name = match key {
            Expr::Str(s) => Some(s.clone()),
            _ => None,
        };
        if let Expr::Name(n) = base {
            let base_ty = self.lookup(n).cloned();
            match base_ty {
                Some(Type::Table(mut tt)) => {
                    match (&key_name, key) {
                        (Some(k), _) => {
                            if let Some(slot) = tt.fields.iter_mut().find(|(fk, _)| fk == k) {
                                slot.1 = val;
                            } else {
                                tt.fields.push((k.clone(), val));
                            }
                        }
                        (None, Expr::Int(_) | Expr::Float(_)) => {
                            let merged = match tt.array.take() {
                                Some(elem) => Type::union_of(vec![*elem, val]),
                                None => val,
                            };
                            tt.array = Some(Box::new(merged));
                        }
                        _ => {
                            self.eval(key);
                        }
                    }
                    self.set_existing(n, Type::Table(tt.clone()));
                    self.update_binding_type(n, Type::Table(tt));
                    return;
                }
                Some(Type::Instance(class)) => {
                    if let Some(k) = key_name {
                        self.add_instance_field(&class, &k, val);
                    }
                    return;
                }
                Some(Type::Class(class)) => {
                    if let Some(k) = key_name {
                        self.add_static_field(&class, &k, val);
                    }
                    return;
                }
                other => {
                    if self.opts.ambient
                        && matches!(other, None | Some(Type::Unknown) | Some(Type::Nil))
                    {
                        if let Some(k) = &key_name {
                            let tt = crate::types::TableType {
                                fields: vec![(k.clone(), val)],
                                array: None,
                                name: Some(n.clone()),
                            };
                            self.bind(n, Type::Table(tt.clone()), true);
                            self.out.bindings.push(Binding {
                                name: n.clone(),
                                line: None,
                                ty: Type::Table(tt),
                                kind: BindingKind::BareAssign,
                            });
                            return;
                        }
                    }
                }
            }
        }
        let bty = self.eval(base);
        match bty {
            Type::Instance(class) => {
                if let Some(k) = key_name {
                    self.add_instance_field(&class, &k, val);
                }
            }
            Type::Class(class) => {
                if let Some(k) = key_name {
                    self.add_static_field(&class, &k, val);
                }
            }
            _ => {
                self.eval(key);
            }
        }
    }

    fn update_binding_type(&mut self, name: &str, ty: Type) {
        if let Some(b) = self
            .out
            .bindings
            .iter_mut()
            .rev()
            .find(|b| b.name == name)
        {
            b.ty = ty;
        }
    }

    fn add_instance_field(&mut self, class: &str, name: &str, ty: Type) {
        let known = self
            .walk_class_chain(class, true, &|info: &ClassInfo| {
                let hit = info.fields.iter().any(|f| f.name == name)
                    || info.getters.iter().any(|g| g.name == name)
                    || info.setters.iter().any(|(n, _)| n == name)
                    || info.methods.iter().any(|m| m.name == name);
                if hit { Some(()) } else { None }
            })
            .is_some();
        if let Some(info) = self.out.classes.get_mut(class) {
            if !known {
                info.fields.push(FieldInfo {
                    name: name.to_string(),
                    is_static: false,
                    ty,
                    access: Access::Public,
                });
            } else if let Some(field) = info
                .fields
                .iter_mut()
                .find(|f| f.name == name && !f.is_static)
            {
                if field.ty == Type::Unknown {
                    field.ty = ty;
                }
            }
        }
    }

    fn add_static_field(&mut self, class: &str, name: &str, ty: Type) {
        if let Some(info) = self.out.classes.get_mut(class) {
            match info
                .fields
                .iter_mut()
                .find(|f| f.name == name && f.is_static)
            {
                Some(field) => field.ty = ty,
                None => info.fields.push(FieldInfo {
                    name: name.to_string(),
                    is_static: true,
                    ty,
                    access: Access::Public,
                }),
            }
        }
    }

    fn for_in_types(&mut self, names: &[String], iters: &[Expr]) -> Vec<Type> {
        let mut out = vec![Type::Unknown; names.len()];
        if iters.len() == 1 {
            if let Expr::Call { callee, args } = &iters[0] {
                if let (Expr::Name(f), [arg]) = (callee.as_ref(), args.as_slice()) {
                    let arg_ty = self.eval(arg);
                    if let Type::Table(tt) = &arg_ty {
                        match f.as_str() {
                            "ipairs" => {
                                if !out.is_empty() {
                                    out[0] = Type::Number;
                                }
                                if out.len() > 1 {
                                    out[1] = tt
                                        .array
                                        .as_ref()
                                        .map(|e| (**e).clone())
                                        .unwrap_or(Type::Unknown);
                                }
                                return out;
                            }
                            "pairs" => {
                                let mut key_parts = Vec::new();
                                let mut val_parts = Vec::new();
                                if !tt.fields.is_empty() {
                                    key_parts.push(Type::String);
                                    val_parts
                                        .extend(tt.fields.iter().map(|(_, t)| t.clone()));
                                }
                                if let Some(elem) = &tt.array {
                                    key_parts.push(Type::Number);
                                    val_parts.push((**elem).clone());
                                }
                                if !out.is_empty() && !key_parts.is_empty() {
                                    out[0] = Type::union_of(key_parts);
                                }
                                if out.len() > 1 && !val_parts.is_empty() {
                                    out[1] = Type::union_of(val_parts);
                                }
                                return out;
                            }
                            _ => {}
                        }
                    }
                    return out;
                }
            }
        }
        for it in iters {
            self.eval(it);
        }
        out
    }
}
