
use std::cell::RefCell;
use std::rc::Rc;

use std::collections::{HashMap, HashSet};

use super::env::{Environment, VarError};
use super::gc;
use super::value::{values_equal, Class, FieldDef, Function, Interface, Key, Native, NativeFn, Table, Value};
use crate::ast::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError(pub String);

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "eval error: {}", self.0)
    }
}

impl std::error::Error for EvalError {}

impl From<VarError> for EvalError {
    fn from(e: VarError) -> Self {
        EvalError(e.to_string())
    }
}

type Result<T> = std::result::Result<T, EvalError>;

#[derive(Debug, Clone)]
enum Flow {
    Normal,
    Break,
    Return(Vec<Value>),
}

pub struct Interpreter {
    pub env: Environment,

    varargs: Vec<Vec<Value>>,

    class_ctx: Vec<Rc<Class>>,

    behaviours: Vec<Rc<Class>>,

    module_dir: std::path::PathBuf,
}

const PRELUDE: &str = r#"
pub class MonoBehaviour {
  function Awake(): void end
  function Start(): void end
  function Update(): void end
  function OnDestroy(): void end
}
"#;

impl Default for Interpreter {
    fn default() -> Self {
        Interpreter::new()
    }
}

impl Interpreter {

    pub fn new() -> Self {
        let mut env = Environment::new();
        register_builtins(&mut env);
        let mut interp = Interpreter {
            env,
            varargs: Vec::new(),
            class_ctx: Vec::new(),
            behaviours: Vec::new(),
            module_dir: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        };
        interp.load_prelude();
        interp
    }

    pub fn set_module_dir(&mut self, dir: impl Into<std::path::PathBuf>) {
        self.module_dir = dir.into();
    }

    fn load_prelude(&mut self) {
        let tokens = crate::lexer::tokenize(PRELUDE).expect("prelude lexes");
        let program = crate::parser::parse(tokens).expect("prelude parses");
        self.run(&program).expect("prelude runs");
    }

    pub(crate) fn with_shared_global(global: super::env::ScopeRef) -> Self {
        Interpreter {
            env: Environment::with_global(global),
            varargs: Vec::new(),
            class_ctx: Vec::new(),
            behaviours: Vec::new(),
            module_dir: std::path::PathBuf::from("."),
        }
    }

    pub fn run(&mut self, program: &[Stmt]) -> Result<Vec<Value>> {
        for stmt in program {
            match self.exec(stmt)? {
                Flow::Normal => {}
                Flow::Break => break,
                Flow::Return(values) => return Ok(values),
            }
            if gc::should_collect() {
                let roots = self.env.gc_roots();
                gc::collect(&roots);
            }
        }
        Ok(Vec::new())
    }

    pub fn collect_garbage(&mut self) {
        let roots = self.env.gc_roots();
        gc::collect(&roots);
    }

    fn exec_block(&mut self, body: &[Stmt]) -> Result<Flow> {
        self.env.push_scope();
        let mut flow = Flow::Normal;
        let mut error = None;
        for stmt in body {
            match self.exec(stmt) {
                Ok(Flow::Normal) => {}

                Ok(other) => {
                    flow = other;
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        self.env.pop_scope();
        match error {
            Some(e) => Err(e),
            None => Ok(flow),
        }
    }

    fn exec(&mut self, stmt: &Stmt) -> Result<Flow> {
        match stmt {
            Stmt::Declare { visibility, mutability, names, inits, .. } => {
                let values = self.eval_values(inits)?;
                for (i, name) in names.iter().enumerate() {
                    let v = values.get(i).cloned().unwrap_or(Value::Nil);
                    self.env.declare(name.clone(), v, *mutability, *visibility);
                }
            }
            Stmt::Assign { targets, op, values, .. } => self.exec_assign(targets, *op, values)?,
            Stmt::Do(body) => return self.exec_block(body),
            Stmt::If { branches, else_block, .. } => {
                for (cond, body) in branches {
                    if self.eval(cond)?.is_truthy() {
                        return self.exec_block(body);
                    }
                }
                if let Some(body) = else_block {
                    return self.exec_block(body);
                }
            }
            Stmt::While { cond, body, .. } => {
                while self.eval(cond)?.is_truthy() {
                    match self.exec_block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Normal => {}
                    }
                }
            }
            Stmt::ForNumeric { var, start, stop, step, body } => {
                return self.exec_for_numeric(var, start, stop, step.as_ref(), body);
            }
            Stmt::ForIn { names, iters, body } => {
                return self.exec_for_in(names, iters, body);
            }
            Stmt::Break { .. } => return Ok(Flow::Break),
            Stmt::Return { values, .. } => {
                let values = self.eval_values(values)?;
                return Ok(Flow::Return(values));
            }

            Stmt::TypeAlias { .. } => {}
            Stmt::Enum { visibility, name, variants, .. } => {
                self.exec_enum(*visibility, name, variants)?;
            }
            Stmt::Class { visibility, is_final, is_abstract, name, parent, mixins, interfaces, members } => {
                let class = self.build_class(name, *is_final, *is_abstract, parent.as_deref(), mixins, interfaces, members)?;

                if !*is_abstract && is_behaviour(&class) {
                    self.behaviours.push(class.clone());
                }
                self.env.declare(name.clone(), Value::Class(class), Mutability::Const, *visibility);
            }
            Stmt::Interface { visibility, name, parents, members } => {
                let iface = self.build_interface(name, parents, members)?;
                self.env.declare(name.clone(), Value::Interface(iface), Mutability::Const, *visibility);
            }
            Stmt::Expr(expr, _) => {
                self.eval(expr)?;
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_assign(&mut self, targets: &[LValue], op: AssignOp, values: &[Expr]) -> Result<()> {

        if op != AssignOp::Assign {
            let rhs_expr = &values[0];
            match &targets[0] {
                LValue::Name(name) => {
                    let current = self
                        .env
                        .get(name)
                        .ok_or_else(|| EvalError(format!("undefined variable '{name}'")))?;
                    let rhs = self.eval(rhs_expr)?;
                    let new_value = self.eval_binop(compound_binop(op), current, rhs)?;
                    self.env.assign(name, new_value)?;
                }
                LValue::Index { base, key } => {
                    let base_val = self.eval(base)?;
                    let key_val = self.eval(key)?;
                    if let Value::Class(c) = &base_val {
                        let current = class_static_get(c, &key_val);
                        let rhs = self.eval(rhs_expr)?;
                        let new_value = self.eval_binop(compound_binop(op), current, rhs)?;
                        c.statics.borrow_mut().set(key_val, new_value).map_err(EvalError)?;
                        return Ok(());
                    }

                    if let (Some(class), Value::Str(k)) = (instance_class(&base_val), &key_val) {
                        self.check_access(&class, k)?;
                        let has_accessor = class.find_getter(k).is_some() || class.find_setter(k).is_some();
                        if has_accessor {
                            let current = match class.find_getter(k) {
                                Some((g, gd)) => {
                                    self.invoke_method(g, base_val.clone(), gd, Vec::new())?.into_iter().next().unwrap_or(Value::Nil)
                                }
                                None => base_val.field(&key_val),
                            };
                            let rhs = self.eval(rhs_expr)?;
                            let new_value = self.eval_binop(compound_binop(op), current, rhs)?;
                            match class.find_setter(k) {
                                Some((s, sd)) => {
                                    self.invoke_method(s, base_val.clone(), sd, vec![new_value])?;
                                }
                                None => base_val.set_field(key_val, new_value).map_err(EvalError)?,
                            }
                            return Ok(());
                        }
                    }
                    let Value::Table(table) = base_val else {
                        return Err(EvalError(index_error(&base_val, &key_val)));
                    };
                    let current = self.index_get(table.clone(), key_val.clone())?;
                    let rhs = self.eval(rhs_expr)?;
                    let new_value = self.eval_binop(compound_binop(op), current, rhs)?;
                    self.index_set(table, key_val, new_value)?;
                }
            }
            return Ok(());
        }

        let vals = self.eval_values(values)?;
        for (i, target) in targets.iter().enumerate() {
            let v = vals.get(i).cloned().unwrap_or(Value::Nil);
            match target {
                LValue::Name(name) => {

                    if self.env.contains(name) {
                        self.env.assign(name, v)?;
                    } else {
                        self.env.declare(name.clone(), v, Mutability::Const, Visibility::Local);
                    }
                }
                LValue::Index { base, key } => {
                    let base_val = self.eval(base)?;
                    let key_val = self.eval(key)?;
                    if let Value::Class(c) = &base_val {
                        c.statics.borrow_mut().set(key_val, v).map_err(EvalError)?;
                        continue;
                    }

                    if let (Some(class), Value::Str(k)) = (instance_class(&base_val), &key_val) {
                        self.check_access(&class, k)?;
                        if let Some((s, defining)) = class.find_setter(k) {
                            self.invoke_method(s, base_val.clone(), defining, vec![v])?;
                            continue;
                        }
                    }
                    let Value::Table(table) = base_val else {
                        return Err(EvalError(index_error(&base_val, &key_val)));
                    };
                    self.index_set(table, key_val, v)?;
                }
            }
        }
        Ok(())
    }

    fn exec_for_numeric(
        &mut self,
        var: &str,
        start: &Expr,
        stop: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
    ) -> Result<Flow> {
        let start = self.eval(start)?;
        let stop = self.eval(stop)?;
        let step = match step {
            Some(e) => self.eval(e)?,
            None => Value::Int(1),
        };
        let start = loop_number(&start)?;
        let stop = loop_number(&stop)?;
        let step = loop_number(&step)?;
        if step == 0.0 {
            return Err(EvalError("'for' step must not be zero".into()));
        }

        let mut i = start;
        loop {
            let keep_going = if step > 0.0 { i <= stop } else { i >= stop };
            if !keep_going {
                break;
            }
            self.env.push_scope();
            self.env.declare(var.to_string(), float_to_value(i), Mutability::Mutable, Visibility::Local);
            let flow = self.exec_block(body);
            self.env.pop_scope();
            match flow? {
                Flow::Break => break,
                Flow::Return(v) => return Ok(Flow::Return(v)),
                Flow::Normal => {}
            }
            i += step;
        }
        Ok(Flow::Normal)
    }

    fn exec_for_in(&mut self, names: &[String], iters: &[Expr], body: &[Stmt]) -> Result<Flow> {

        let state = self.eval_values(iters)?;
        let iter_fn = state.first().cloned().unwrap_or(Value::Nil);
        let iter_state = state.get(1).cloned().unwrap_or(Value::Nil);
        let mut control = state.get(2).cloned().unwrap_or(Value::Nil);

        loop {
            let results = self.call(&iter_fn, vec![iter_state.clone(), control.clone()])?;
            let first = results.first().cloned().unwrap_or(Value::Nil);
            if matches!(first, Value::Nil) {
                break;
            }
            control = first;

            self.env.push_scope();
            for (i, name) in names.iter().enumerate() {
                let v = results.get(i).cloned().unwrap_or(Value::Nil);
                self.env.declare(name.clone(), v, Mutability::Mutable, Visibility::Local);
            }
            let flow = self.exec_block(body);
            self.env.pop_scope();
            match flow? {
                Flow::Break => break,
                Flow::Return(v) => return Ok(Flow::Return(v)),
                Flow::Normal => {}
            }
        }
        Ok(Flow::Normal)
    }

    fn eval_values(&mut self, exprs: &[Expr]) -> Result<Vec<Value>> {
        if exprs.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(exprs.len());
        let last = exprs.len() - 1;
        for (i, e) in exprs.iter().enumerate() {

            if i == last {
                match e {
                    Expr::Call { callee, args } => {
                        let callee_val = self.eval(callee)?;
                        let mut argv = Vec::with_capacity(args.len());
                        for a in args {
                            argv.push(self.eval(a)?);
                        }
                        out.extend(self.call(&callee_val, argv)?);
                    }
                    Expr::MethodCall { .. } => out.extend(self.eval_method_call(e)?),
                    Expr::Vararg => {
                        if let Some(va) = self.varargs.last() {
                            out.extend(va.clone());
                        }
                    }
                    _ => out.push(self.eval(e)?),
                }
            } else {
                out.push(self.eval(e)?);
            }
        }
        Ok(out)
    }

    fn eval(&mut self, expr: &Expr) -> Result<Value> {
        match expr {
            Expr::Nil => Ok(Value::Nil),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Int(i) => Ok(Value::Int(*i)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::Str(s) => Ok(Value::str(s.as_str())),
            Expr::Name(name) => self
                .env
                .get(name)
                .ok_or_else(|| EvalError(format!("undefined variable '{name}'"))),
            Expr::Table(entries) => self.eval_table(entries),
            Expr::Index { base, key } => {
                let base_val = self.eval(base)?;
                let key_val = self.eval(key)?;
                match &base_val {
                    Value::Table(table) => {

                        if let (Some(class), Value::Str(k)) = (instance_class(&base_val), &key_val) {
                            self.check_access(&class, k)?;
                            if let Some((g, defining)) = class.find_getter(k) {
                                let res = self.invoke_method(g, base_val.clone(), defining, Vec::new())?;
                                return Ok(res.into_iter().next().unwrap_or(Value::Nil));
                            }
                        }
                        self.index_get(table.clone(), key_val)
                    }

                    Value::Class(c) => Ok(class_static_get(c, &key_val)),
                    other => Err(EvalError(index_error(other, &key_val))),
                }
            }
            Expr::Call { callee, args } => {
                let callee_val = self.eval(callee)?;
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(self.eval(a)?);
                }
                self.call_one(&callee_val, arg_vals)
            }
            Expr::MethodCall { .. } => {
                Ok(self.eval_method_call(expr)?.into_iter().next().unwrap_or(Value::Nil))
            }
            Expr::Vararg => {
                Ok(self.varargs.last().and_then(|v| v.first().cloned()).unwrap_or(Value::Nil))
            }
            Expr::Function { name, params, is_vararg, body } => Ok(Value::function(
                name.clone(),
                params.clone(),
                *is_vararg,
                Rc::new(body.clone()),
                self.env.capture(),
            )),
            Expr::Switch { subject, cases, default } => {
                let subj = self.eval(subject)?;
                for case in cases {
                    let pat = self.eval(&case.pattern)?;
                    if values_equal(&subj, &pat) {
                        return self.run_switch_body(&case.body);
                    }
                }
                match default {
                    Some(body) => self.run_switch_body(body),
                    None => Ok(Value::Nil),
                }
            }
            Expr::Unary { op, expr } => {
                let v = self.eval(expr)?;

                if let Some(class) = instance_class(&v) {
                    let mm = match op {
                        UnaryOp::Neg => Some("__unm"),
                        UnaryOp::Len => Some("__len"),
                        UnaryOp::Not => None,
                    };
                    if let Some(mm) = mm {
                        if let Some((f, defining)) = class.find_operator(mm) {
                            let res = self.invoke_method(f, v.clone(), defining, Vec::new())?;
                            return Ok(res.into_iter().next().unwrap_or(Value::Nil));
                        }
                    }
                }

                if let Value::Table(t) = &v {
                    let mm = match op {
                        UnaryOp::Neg => Some("__unm"),
                        UnaryOp::Len => Some("__len"),
                        UnaryOp::Not => None,
                    };
                    if let Some(mm) = mm {
                        let callable = t.borrow().metamethod(mm);
                        if let Some(callable) = callable {
                            return self.call_one(&callable, vec![v.clone()]);
                        }
                    }
                }
                eval_unary(*op, v)
            }
            Expr::Binary { op, lhs, rhs } => {
                let a = self.eval(lhs)?;
                let b = self.eval(rhs)?;
                self.eval_binop(*op, a, b)
            }
            Expr::Logical { op, lhs, rhs } => {
                let a = self.eval(lhs)?;

                match op {
                    LogicalOp::And if !a.is_truthy() => Ok(a),
                    LogicalOp::Or if a.is_truthy() => Ok(a),
                    _ => self.eval(rhs),
                }
            }
        }
    }

    fn run_switch_body(&mut self, body: &[Stmt]) -> Result<Value> {
        self.env.push_scope();
        let mut result = Value::Nil;
        let mut error = None;
        for stmt in body {
            match self.exec(stmt) {
                Ok(Flow::Normal) => {}
                Ok(Flow::Return(vals)) => {
                    result = vals.into_iter().next().unwrap_or(Value::Nil);
                    break;
                }
                Ok(Flow::Break) => break,
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        self.env.pop_scope();
        match error {
            Some(e) => Err(e),
            None => Ok(result),
        }
    }

    fn eval_table(&mut self, entries: &[TableEntry]) -> Result<Value> {
        let table = Value::table();
        let Value::Table(rc) = &table else { unreachable!() };
        let last = entries.len().wrapping_sub(1);
        for (idx, entry) in entries.iter().enumerate() {
            match entry {

                TableEntry::Positional(e) if idx == last => {
                    for v in self.eval_values(std::slice::from_ref(e))? {
                        rc.borrow_mut().array.push(v);
                    }
                }
                TableEntry::Positional(e) => {
                    let v = self.eval(e)?;
                    rc.borrow_mut().array.push(v);
                }
                TableEntry::Keyed { key, value } => {
                    let k = self.eval(key)?;
                    let v = self.eval(value)?;
                    rc.borrow_mut().set(k, v).map_err(EvalError)?;
                }
            }
        }
        Ok(table)
    }

    fn index_get(&mut self, mut table: Rc<RefCell<Table>>, key: Value) -> Result<Value> {
        loop {
            let raw = table.borrow().get(&key);
            if !matches!(raw, Value::Nil) {
                return Ok(raw);
            }
            let meta = table.borrow().metamethod("__index");
            match meta {
                None => return Ok(Value::Nil),
                Some(Value::Table(next)) => table = next,
                Some(callable) => {
                    return self.call_one(&callable, vec![Value::Table(table.clone()), key]);
                }
            }
        }
    }

    fn index_set(&mut self, table: Rc<RefCell<Table>>, key: Value, value: Value) -> Result<()> {
        let present = !matches!(table.borrow().get(&key), Value::Nil);
        if present {
            table.borrow_mut().set(key, value).map_err(EvalError)?;
            return Ok(());
        }
        let meta = table.borrow().metamethod("__newindex");
        match meta {
            None => table.borrow_mut().set(key, value).map_err(EvalError)?,
            Some(Value::Table(next)) => return self.index_set(next, key, value),
            Some(callable) => {
                self.call(&callable, vec![Value::Table(table.clone()), key, value])?;
            }
        }
        Ok(())
    }

    pub(crate) fn call(&mut self, callee: &Value, args: Vec<Value>) -> Result<Vec<Value>> {
        match callee {
            Value::Native(n) => {
                let func = n.func;
                func(self, args).map_err(EvalError)
            }
            Value::Function(f) => self.invoke(&f.clone(), args, None),

            Value::Class(c) => self.construct(c.clone(), args),

            Value::Table(t) => {
                let mm = t.borrow().metamethod("__call");
                match mm {
                    Some(callable) => {
                        let mut full = Vec::with_capacity(args.len() + 1);
                        full.push(callee.clone());
                        full.extend(args);
                        self.call(&callable, full)
                    }
                    None => Err(EvalError("attempt to call a table".into())),
                }
            }
            other => Err(EvalError(format!("attempt to call a {} value", other.type_name()))),
        }
    }

    pub(crate) fn call_one(&mut self, callee: &Value, args: Vec<Value>) -> Result<Value> {
        Ok(self.call(callee, args)?.into_iter().next().unwrap_or(Value::Nil))
    }

    fn invoke(
        &mut self,
        f: &Rc<Function>,
        args: Vec<Value>,
        method: Option<(Value, Value, Rc<Class>)>,
    ) -> Result<Vec<Value>> {
        let saved = self.env.swap_current(f.captured.clone());
        self.env.push_scope();

        let is_method = method.is_some();
        if let Some((self_v, super_v, class)) = method {
            self.env.declare("self".to_string(), self_v, Mutability::Const, Visibility::Local);
            self.env.declare("super".to_string(), super_v, Mutability::Const, Visibility::Local);
            self.class_ctx.push(class);
        }

        for (i, param) in f.params.iter().enumerate() {
            let v = args.get(i).cloned().unwrap_or(Value::Nil);
            self.env.declare(param.clone(), v, Mutability::Mutable, Visibility::Local);
        }
        if f.is_vararg {
            let extra = if args.len() > f.params.len() { args[f.params.len()..].to_vec() } else { Vec::new() };
            self.varargs.push(extra);
        }

        let mut result = Vec::new();
        let mut error = None;
        for stmt in f.body.iter() {
            match self.exec(stmt) {
                Ok(Flow::Normal) | Ok(Flow::Break) => {}
                Ok(Flow::Return(values)) => {
                    result = values;
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }

        if f.is_vararg {
            self.varargs.pop();
        }
        if is_method {
            self.class_ctx.pop();
        }
        self.env.pop_scope();
        self.env.swap_current(saved);
        match error {
            Some(e) => Err(e),
            None => Ok(result),
        }
    }

    fn eval_binop(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value> {
        use BinOp::*;

        if instance_class(&a).is_some() || instance_class(&b).is_some() {
            if let Some(result) = self.try_class_binop(op, &a, &b)? {
                return Ok(result);
            }
        }
        if matches!(a, Value::Table(_)) || matches!(b, Value::Table(_)) {
            if let Some(name) = operator_metamethod(op) {

                let applies = !matches!(op, Eq | Ne)
                    || (matches!(a, Value::Table(_)) && matches!(b, Value::Table(_)));
                if applies {
                    if let Some(mm) = get_metamethod(&a, name).or_else(|| get_metamethod(&b, name)) {
                        let result = self.call_one(&mm, vec![a.clone(), b.clone()])?;
                        return Ok(match op {
                            Ne => Value::Bool(!result.is_truthy()),
                            _ => result,
                        });
                    }
                }
            }
        }
        apply_binop(op, a, b)
    }

    fn exec_enum(
        &mut self,
        visibility: Visibility,
        name: &str,
        variants: &[(String, Option<Expr>)],
    ) -> Result<()> {

        let table = match self.env.get(name) {
            Some(Value::Table(rc)) => Value::Table(rc),
            _ => {
                let v = Value::table();
                match visibility {
                    Visibility::Pub => {
                        self.env.declare(name.to_string(), v.clone(), Mutability::Const, Visibility::Pub)
                    }
                    Visibility::Local => {
                        self.env.declare_module_global(name.to_string(), v.clone(), Mutability::Const)
                    }
                }
                v
            }
        };
        let Value::Table(rc) = &table else { unreachable!() };

        let mut counter = rc
            .borrow()
            .map
            .values()
            .filter_map(|v| if let Value::Int(n) = v { Some(*n) } else { None })
            .max()
            .map_or(0, |m| m + 1);

        for (vname, value) in variants {
            let v = match value {
                Some(e) => self.eval(e)?,
                None => Value::Int(counter),
            };
            if let Value::Int(n) = &v {
                counter = n + 1;
            } else {
                counter += 1;
            }
            rc.borrow_mut().set(Value::str(vname.as_str()), v).map_err(EvalError)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_class(
        &mut self,
        name: &str,
        is_final: bool,
        is_abstract: bool,
        parent: Option<&str>,
        mixin_names: &[String],
        interface_names: &[String],
        members: &[ClassMember],
    ) -> Result<Rc<Class>> {
        let parent_class = match parent {
            Some(p) => match self.env.get(p) {
                Some(Value::Class(c)) => Some(c),
                Some(_) => return Err(EvalError(format!("'{p}' is not a class"))),
                None => return Err(EvalError(format!("unknown parent class '{p}'"))),
            },
            None => None,
        };
        if let Some(pc) = &parent_class {
            if pc.is_final {
                return Err(EvalError(format!("cannot extend final class '{}'", pc.name)));
            }
        }

        let mut mixin_classes: Vec<Rc<Class>> = Vec::new();
        for mname in mixin_names {
            match self.env.get(mname) {
                Some(Value::Class(c)) => mixin_classes.push(c),
                Some(_) => return Err(EvalError(format!("'{mname}' is not a class (mixin)"))),
                None => return Err(EvalError(format!("unknown mixin class '{mname}'"))),
            }
        }

        let mut interfaces: Vec<Rc<Interface>> = Vec::new();
        for iname in interface_names {
            match self.env.get(iname) {
                Some(Value::Interface(i)) => interfaces.push(i),
                Some(_) => return Err(EvalError(format!("'{iname}' is not an interface"))),
                None => return Err(EvalError(format!("unknown interface '{iname}'"))),
            }
        }

        let captured = self.env.capture();
        let mut methods: HashMap<String, Value> = HashMap::new();
        let mut operators: HashMap<String, Value> = HashMap::new();
        let mut getters: HashMap<String, Value> = HashMap::new();
        let mut setters: HashMap<String, Value> = HashMap::new();
        let mut constructor: Option<Value> = None;
        let mut fields: Vec<FieldDef> = Vec::new();
        let mut access_map: HashMap<String, Access> = HashMap::new();
        let mut abstracts: HashSet<String> = HashSet::new();
        let mut finals: HashSet<String> = HashSet::new();
        let statics = Value::table();
        let Value::Table(statics_rc) = &statics else { unreachable!() };

        if let Some(pc) = &parent_class {
            for f in &pc.fields {
                fields.push(FieldDef { name: f.name.clone(), default: f.default.clone() });
            }
        }

        for mx in &mixin_classes {
            for (mname, mval) in &mx.methods {
                methods.insert(mname.clone(), mval.clone());
            }
            for (op, oval) in &mx.operators {
                operators.insert(op.clone(), oval.clone());
            }
            for f in &mx.fields {
                if !fields.iter().any(|x| x.name == f.name) {
                    fields.push(FieldDef { name: f.name.clone(), default: f.default.clone() });
                }
            }
        }

        for m in members {
            match m {
                ClassMember::Field { access, is_static, name: fname, default } => {
                    if *access != Access::Public {
                        access_map.insert(fname.clone(), *access);
                    }
                    if *is_static {
                        let v = match default {
                            Some(e) => self.eval(e)?,
                            None => Value::Nil,
                        };
                        statics_rc.borrow_mut().set(Value::str(fname.as_str()), v).map_err(EvalError)?;
                    } else {
                        fields.retain(|f| f.name != *fname);
                        fields.push(FieldDef { name: fname.clone(), default: default.clone() });
                    }
                }
                ClassMember::Method { access, is_static, is_abstract, is_final: mfinal, is_override, name: mname, func } => {
                    if *access != Access::Public {
                        access_map.insert(mname.clone(), *access);
                    }
                    if *is_abstract {
                        abstracts.insert(mname.clone());
                    }
                    if *mfinal {
                        finals.insert(mname.clone());
                    }
                    if let Some(pc) = &parent_class {
                        if pc.has_final_method(mname) {
                            return Err(EvalError(format!("cannot override final method '{mname}'")));
                        }
                    }

                    if *is_override {
                        let inherited = parent_class.as_ref().is_some_and(|pc| pc.find_method(mname).is_some())
                            || mixin_classes.iter().any(|mx| mx.methods.contains_key(mname));
                        if !inherited {
                            return Err(EvalError(format!(
                                "method '{mname}' is marked `override` but does not override anything"
                            )));
                        }
                    }
                    let fval = Value::function(
                        mname.clone(),
                        func.params.clone(),
                        func.is_vararg,
                        Rc::new(func.body.clone()),
                        captured.clone(),
                    );
                    if *is_static {
                        statics_rc.borrow_mut().set(Value::str(mname.as_str()), fval).map_err(EvalError)?;
                    } else {
                        methods.insert(mname.clone(), fval);
                    }
                }
                ClassMember::Getter { access, name: gname, func } => {
                    if *access != Access::Public {
                        access_map.insert(gname.clone(), *access);
                    }
                    getters.insert(
                        gname.clone(),
                        Value::function(format!("get {gname}"), func.params.clone(), func.is_vararg, Rc::new(func.body.clone()), captured.clone()),
                    );
                }
                ClassMember::Setter { access, name: sname, func } => {
                    if *access != Access::Public {
                        access_map.insert(sname.clone(), *access);
                    }
                    setters.insert(
                        sname.clone(),
                        Value::function(format!("set {sname}"), func.params.clone(), func.is_vararg, Rc::new(func.body.clone()), captured.clone()),
                    );
                }
                ClassMember::Constructor { func } => {
                    constructor = Some(Value::function(
                        "constructor".to_string(),
                        func.params.clone(),
                        func.is_vararg,
                        Rc::new(func.body.clone()),
                        captured.clone(),
                    ));
                }
                ClassMember::Operator { symbol, func } => {
                    let mm = operator_to_metamethod(symbol, func.params.len())
                        .ok_or_else(|| EvalError(format!("unsupported operator overload '{symbol}'")))?;
                    let fval = Value::function(
                        format!("operator{symbol}"),
                        func.params.clone(),
                        func.is_vararg,
                        Rc::new(func.body.clone()),
                        captured.clone(),
                    );
                    operators.insert(mm.to_string(), fval);
                }
            }
        }

        let instance_meta = Value::table();
        let Value::Table(meta_rc) = &instance_meta else { unreachable!() };
        let class = Rc::new(Class {
            name: name.to_string(),
            parent: parent_class,
            methods,
            operators,
            getters,
            setters,
            constructor,
            fields,
            statics: statics_rc.clone(),
            access: access_map,
            abstracts,
            finals,
            is_final,
            is_abstract,
            interfaces,
            instance_meta: meta_rc.clone(),
            gc_mark: std::cell::Cell::new(false),
        });
        gc::register_class(&class);

        meta_rc.borrow_mut().set(Value::str("__class"), Value::Class(class.clone())).map_err(EvalError)?;

        for iface in &class.interfaces {
            for member in &iface.members {
                if !class.has_member(member) {
                    return Err(EvalError(format!(
                        "class '{}' does not implement member '{member}' required by interface '{}'",
                        class.name, iface.name
                    )));
                }
            }
        }
        Ok(class)
    }

    fn build_interface(&mut self, name: &str, parents: &[String], members: &[String]) -> Result<Rc<Interface>> {
        let mut parent_ifaces: Vec<Rc<Interface>> = Vec::new();
        let mut all: HashSet<String> = members.iter().cloned().collect();
        for pname in parents {
            match self.env.get(pname) {
                Some(Value::Interface(p)) => {
                    all.extend(p.members.iter().cloned());
                    parent_ifaces.push(p);
                }
                Some(_) => return Err(EvalError(format!("'{pname}' is not an interface"))),
                None => return Err(EvalError(format!("unknown interface '{pname}'"))),
            }
        }
        Ok(Rc::new(Interface { name: name.to_string(), members: all, parents: parent_ifaces }))
    }

    fn construct(&mut self, class: Rc<Class>, args: Vec<Value>) -> Result<Vec<Value>> {
        if class.is_abstract {
            return Err(EvalError(format!("cannot instantiate abstract class '{}'", class.name)));
        }
        let inst = Value::table();
        inst.set_metatable(Value::Table(class.instance_meta.clone())).map_err(EvalError)?;
        let Value::Table(inst_rc) = &inst else { unreachable!() };

        self.env.push_scope();
        self.env.declare("self".to_string(), inst.clone(), Mutability::Const, Visibility::Local);
        self.class_ctx.push(class.clone());
        let mut err = None;
        for fd in &class.fields {
            let v = match &fd.default {
                Some(e) => match self.eval(e) {
                    Ok(v) => v,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                },
                None => Value::Nil,
            };
            if let Err(e) = inst_rc.borrow_mut().set(Value::str(fd.name.as_str()), v).map_err(EvalError) {
                err = Some(e);
                break;
            }
        }
        self.class_ctx.pop();
        self.env.pop_scope();
        if let Some(e) = err {
            return Err(e);
        }

        if let Some((ctor, defining)) = find_constructor(&class) {
            match &ctor {
                Value::Function(f) => {
                    let super_v = defining.parent.clone().map(Value::Class).unwrap_or(Value::Nil);
                    self.invoke(f, args, Some((inst.clone(), super_v, defining)))?;
                }

                Value::Native(n) => {
                    let mut full = Vec::with_capacity(args.len() + 1);
                    full.push(inst.clone());
                    full.extend(args);
                    (n.func)(self, full).map_err(EvalError)?;
                }
                _ => {}
            }
        }
        Ok(vec![inst])
    }

    fn eval_method_call(&mut self, expr: &Expr) -> Result<Vec<Value>> {
        let Expr::MethodCall { receiver, method, args } = expr else { unreachable!() };
        let recv = self.eval(receiver)?;
        let mut argv = Vec::with_capacity(args.len());
        for a in args {
            argv.push(self.eval(a)?);
        }

        if let Value::Class(c) = &recv {
            let c = c.clone();
            let self_v = self.env.get("self").unwrap_or(Value::Nil);

            if method == "constructor" {
                let (ctor, defining) = find_constructor(&c)
                    .ok_or_else(|| EvalError(format!("class '{}' has no constructor", c.name)))?;
                return self.invoke_method(ctor, self_v, defining, argv);
            }
            self.check_access(&c, method)?;
            let (m, defining) = c
                .find_method(method)
                .ok_or_else(|| EvalError(format!("class '{}' has no method '{method}'", c.name)))?;
            check_abstract(&defining, method)?;
            return self.invoke_method(m, self_v, defining, argv);
        }

        if let Some(class) = instance_class(&recv) {
            self.check_access(&class, method)?;
            let (m, defining) = class
                .find_method(method)
                .ok_or_else(|| EvalError(format!("class '{}' has no method '{method}'", class.name)))?;
            check_abstract(&defining, method)?;
            return self.invoke_method(m, recv.clone(), defining, argv);
        }

        if let Value::Table(t) = &recv {
            let m = self.index_get(t.clone(), Value::str(method.as_str()))?;
            let mut full = Vec::with_capacity(argv.len() + 1);
            full.push(recv.clone());
            full.extend(argv);
            return self.call(&m, full);
        }

        Err(EvalError(format!("attempt to call method '{method}' on a {}", recv.type_name())))
    }

    fn call_lifecycle(&mut self, instance: &Value, name: &str) -> Result<()> {
        if let Some(class) = instance_class(instance) {
            if let Some((m, defining)) = class.find_method(name) {
                self.invoke_method(m, instance.clone(), defining, Vec::new())?;
            }
        }
        Ok(())
    }

    fn invoke_method(&mut self, m: Value, self_v: Value, defining: Rc<Class>, args: Vec<Value>) -> Result<Vec<Value>> {
        let super_v = defining.parent.clone().map(Value::Class).unwrap_or(Value::Nil);
        match m {
            Value::Function(f) => self.invoke(&f, args, Some((self_v, super_v, defining))),
            Value::Native(n) => {
                let mut full = Vec::with_capacity(args.len() + 1);
                full.push(self_v);
                full.extend(args);
                (n.func)(self, full).map_err(EvalError)
            }
            other => self.call(&other, args),
        }
    }

    fn display_string(&mut self, v: &Value) -> Result<String> {
        if let Some(class) = instance_class(v) {
            if let Some((f, defining)) = class.find_operator("__tostring") {
                let res = self.invoke_method(f, v.clone(), defining, Vec::new())?;
                return Ok(res.into_iter().next().unwrap_or(Value::Nil).to_string());
            }
            return Ok(format!("{} {v}", class.name));
        }

        if let Value::Table(t) = v {
            let mm = t.borrow().metamethod("__tostring");
            if let Some(callable) = mm {
                let res = self.call_one(&callable, vec![v.clone()])?;
                return Ok(res.to_string());
            }
        }
        Ok(v.to_string())
    }

    fn check_access(&self, class: &Rc<Class>, name: &str) -> Result<()> {
        let Some((access, decl)) = class.member_access(name) else {
            return Ok(());
        };
        let ctx = self.class_ctx.last();
        let allowed = match access {
            Access::Public => true,
            Access::Protected => ctx.is_some_and(|c| c.descends_from(&decl)),
            Access::Private => ctx.is_some_and(|c| Rc::ptr_eq(c, &decl)),
        };
        if !allowed {
            let kind = match access {
                Access::Public => "public",
                Access::Protected => "protected",
                Access::Private => "private",
            };
            return Err(EvalError(format!("member '{name}' is {kind} to class '{}'", decl.name)));
        }
        Ok(())
    }

    fn try_class_binop(&mut self, op: BinOp, a: &Value, b: &Value) -> Result<Option<Value>> {
        use BinOp::*;
        let (mm, swap, negate) = match op {
            Add => ("__add", false, false),
            Sub => ("__sub", false, false),
            Mul => ("__mul", false, false),
            Div => ("__div", false, false),
            Mod => ("__mod", false, false),
            Pow => ("__pow", false, false),
            Concat => ("__concat", false, false),
            Eq => ("__eq", false, false),
            Ne => ("__eq", false, true),
            Lt => ("__lt", false, false),
            Le => ("__le", false, false),
            Gt => ("__lt", true, false),
            Ge => ("__le", true, false),
        };
        let (left, right) = if swap { (b, a) } else { (a, b) };

        let pick = instance_class(left)
            .filter(|c| c.find_operator(mm).is_some())
            .map(|c| (left.clone(), right.clone(), c))
            .or_else(|| {
                instance_class(right)
                    .filter(|c| c.find_operator(mm).is_some())
                    .map(|c| (right.clone(), left.clone(), c))
            });
        let Some((self_v, other_v, class)) = pick else {
            return Ok(None);
        };
        let (f, defining) = class.find_operator(mm).unwrap();
        let res = self.invoke_method(f, self_v, defining, vec![other_v])?;
        let v = res.into_iter().next().unwrap_or(Value::Nil);
        Ok(Some(if negate { Value::Bool(!v.is_truthy()) } else { v }))
    }
}

fn index_error(base: &Value, key: &Value) -> String {
    let what = match key {
        Value::Str(k) => format!(" (field '{k}')"),
        Value::Int(i) => format!(" (index {i})"),
        _ => String::new(),
    };
    format!("attempt to index a {} value{what}", base.type_name())
}

fn instance_class(v: &Value) -> Option<Rc<Class>> {
    if let Value::Table(t) = v {
        let meta = t.borrow().meta.clone()?;
        if let Value::Class(c) = meta.borrow().get(&Value::str("__class")) {
            return Some(c);
        }
    }
    None
}

fn class_static_get(class: &Rc<Class>, key: &Value) -> Value {
    let mut cur = class.clone();
    loop {
        let v = cur.statics.borrow().get(key);
        if !matches!(v, Value::Nil) {
            return v;
        }
        match cur.parent.clone() {
            Some(p) => cur = p,
            None => return Value::Nil,
        }
    }
}

fn check_abstract(defining: &Rc<Class>, method: &str) -> Result<()> {
    if defining.abstracts.contains(method) {
        return Err(EvalError(format!(
            "abstract method '{method}' has no implementation in class '{}'",
            defining.name
        )));
    }
    Ok(())
}

fn find_constructor(class: &Rc<Class>) -> Option<(Value, Rc<Class>)> {
    let mut cur = class.clone();
    loop {
        if let Some(c) = &cur.constructor {
            return Some((c.clone(), cur.clone()));
        }
        cur = cur.parent.clone()?;
    }
}

fn operator_to_metamethod(sym: &str, user_params: usize) -> Option<&'static str> {
    Some(match sym {
        "+" => "__add",
        "-" => {
            if user_params == 0 {
                "__unm"
            } else {
                "__sub"
            }
        }
        "*" => "__mul",
        "/" => "__div",
        "%" => "__mod",
        "^" => "__pow",
        ".." => "__concat",
        "==" => "__eq",
        "<" => "__lt",
        "<=" => "__le",
        "#" => "__len",
        "tostring" => "__tostring",
        _ => return None,
    })
}

fn loop_number(v: &Value) -> Result<f64> {
    coerce_number(v)
        .map(Num::as_f64)
        .ok_or_else(|| EvalError(format!("'for' expects a number, got {}", v.type_name())))
}

fn float_to_value(f: f64) -> Value {
    if f.is_finite() && f.fract() == 0.0 {
        Value::Int(f as i64)
    } else {
        Value::Float(f)
    }
}

fn register_builtins(env: &mut Environment) {
    let builtins: &[(&'static str, NativeFn)] = &[
        ("setmetatable", builtin_setmetatable),
        ("getmetatable", builtin_getmetatable),
        ("type", builtin_type),
        ("print", builtin_print),
        ("rawget", builtin_rawget),
        ("rawset", builtin_rawset),
        ("rawequal", builtin_rawequal),
        ("rawlen", builtin_rawlen),
        ("pcall", builtin_pcall),
        ("ipairs", builtin_ipairs),
        ("pairs", builtin_pairs),
        ("next", builtin_next),
        ("collectgarbage", builtin_collectgarbage),
        ("require", builtin_require),
        ("instanceof", builtin_instanceof),
        ("classname", builtin_classname),
        ("classof", builtin_classof),
        ("superclass", builtin_superclass),
        ("methodsof", builtin_methodsof),
        ("isabstract", builtin_isabstract),
        ("tostring", builtin_tostring),
        ("tonumber", builtin_tonumber),
        ("run", builtin_run),
        ("spawn", builtin_spawn),
    ];
    for (name, func) in builtins {
        let value = Value::Native(Native { name, func: *func });
        env.declare(*name, value, Mutability::Const, Visibility::Pub);
    }

    let coro = Value::table();
    let members: &[(&'static str, NativeFn)] = &[
        ("create", coro_create),
        ("resume", coro_resume),
        ("yield", coro_yield),
        ("status", coro_status),
        ("close", coro_close),
    ];
    for (name, func) in members {
        let _ = coro.set_field(Value::str(*name), Value::Native(Native { name, func: *func }));
    }
    env.declare("coroutine", coro, Mutability::Const, Visibility::Pub);

    let math = Value::table();
    let math_fns: &[(&'static str, NativeFn)] = &[
        ("abs", math_abs),
        ("ceil", math_ceil),
        ("floor", math_floor),
        ("round", math_round),
        ("sqrt", math_sqrt),
        ("sin", math_sin),
        ("cos", math_cos),
        ("tan", math_tan),
        ("asin", math_asin),
        ("acos", math_acos),
        ("atan", math_atan),
        ("exp", math_exp),
        ("log", math_log),
        ("pow", math_pow),
        ("fmod", math_fmod),
        ("modf", math_modf),
        ("max", math_max),
        ("min", math_min),
        ("clamp", math_clamp),
        ("sign", math_sign),
        ("deg", math_deg),
        ("rad", math_rad),
        ("random", math_random),
        ("randomseed", math_randomseed),
    ];
    for (name, func) in math_fns {
        let _ = math.set_field(Value::str(*name), Value::Native(Native { name, func: *func }));
    }
    let _ = math.set_field(Value::str("pi"), Value::Float(std::f64::consts::PI));
    let _ = math.set_field(Value::str("huge"), Value::Float(f64::INFINITY));
    let _ = math.set_field(Value::str("maxinteger"), Value::Int(i64::MAX));
    let _ = math.set_field(Value::str("mininteger"), Value::Int(i64::MIN));
    env.declare("math", math, Mutability::Const, Visibility::Pub);

    register_library(env, "string", &[
        ("len", str_len),
        ("sub", str_sub),
        ("upper", str_upper),
        ("lower", str_lower),
        ("rep", str_rep),
        ("reverse", str_reverse),
        ("byte", str_byte),
        ("char", str_char),
        ("find", str_find),
        ("contains", str_contains),
        ("startswith", str_startswith),
        ("endswith", str_endswith),
        ("trim", str_trim),
        ("split", str_split),
        ("format", str_format),
    ]);

    register_library(env, "table", &[
        ("insert", tbl_insert),
        ("remove", tbl_remove),
        ("concat", tbl_concat),
        ("unpack", tbl_unpack),
        ("pack", tbl_pack),
        ("sort", tbl_sort),
        ("keys", tbl_keys),
    ]);

    register_library(env, "bit32", &[
        ("band", bit_band),
        ("bor", bit_bor),
        ("bxor", bit_bxor),
        ("bnot", bit_bnot),
        ("lshift", bit_lshift),
        ("rshift", bit_rshift),
        ("arshift", bit_arshift),
    ]);

    register_library(env, "os", &[("time", os_time), ("clock", os_clock)]);
}

fn register_library(env: &mut Environment, name: &'static str, fns: &[(&'static str, NativeFn)]) {
    let lib = Value::table();
    for (fname, func) in fns {
        let _ = lib.set_field(Value::str(*fname), Value::Native(Native { name: fname, func: *func }));
    }
    env.declare(name, lib, Mutability::Const, Visibility::Pub);
}

thread_local! {
    static RNG_STATE: std::cell::Cell<u64> = const { std::cell::Cell::new(0x2545F491_4F6CDD1D) };
}

fn next_rand_f64() -> f64 {
    let x = RNG_STATE.with(|r| {
        let mut x = r.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        r.set(x);
        x
    });

    (x >> 11) as f64 / ((1u64 << 53) as f64)
}

fn arg_num(args: &[Value], i: usize, who: &str) -> std::result::Result<f64, String> {
    args.get(i)
        .and_then(coerce_number)
        .map(Num::as_f64)
        .ok_or_else(|| format!("{who}: expected a number"))
}

fn math_abs(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![float_to_value(arg_num(&a, 0, "math.abs")?.abs())])
}
fn math_ceil(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![float_to_value(arg_num(&a, 0, "math.ceil")?.ceil())])
}
fn math_floor(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![float_to_value(arg_num(&a, 0, "math.floor")?.floor())])
}
fn math_round(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![float_to_value(arg_num(&a, 0, "math.round")?.round())])
}
fn math_sqrt(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![float_to_value(arg_num(&a, 0, "math.sqrt")?.sqrt())])
}
fn math_sin(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.sin")?.sin())])
}
fn math_cos(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.cos")?.cos())])
}
fn math_tan(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.tan")?.tan())])
}
fn math_asin(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.asin")?.asin())])
}
fn math_acos(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.acos")?.acos())])
}
fn math_atan(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let y = arg_num(&a, 0, "math.atan")?;

    let r = if a.len() >= 2 { y.atan2(arg_num(&a, 1, "math.atan")?) } else { y.atan() };
    Ok(vec![Value::Float(r)])
}
fn math_exp(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.exp")?.exp())])
}
fn math_log(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let x = arg_num(&a, 0, "math.log")?;
    let r = if a.len() >= 2 { x.log(arg_num(&a, 1, "math.log")?) } else { x.ln() };
    Ok(vec![Value::Float(r)])
}
fn math_pow(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![float_to_value(arg_num(&a, 0, "math.pow")?.powf(arg_num(&a, 1, "math.pow")?))])
}
fn math_fmod(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.fmod")? % arg_num(&a, 1, "math.fmod")?)])
}
fn math_modf(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let x = arg_num(&a, 0, "math.modf")?;
    Ok(vec![float_to_value(x.trunc()), Value::Float(x.fract())])
}
fn math_max(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    if a.is_empty() {
        return Err("math.max: expected at least one number".into());
    }
    let mut best = arg_num(&a, 0, "math.max")?;
    for i in 1..a.len() {
        best = best.max(arg_num(&a, i, "math.max")?);
    }
    Ok(vec![float_to_value(best)])
}
fn math_min(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    if a.is_empty() {
        return Err("math.min: expected at least one number".into());
    }
    let mut best = arg_num(&a, 0, "math.min")?;
    for i in 1..a.len() {
        best = best.min(arg_num(&a, i, "math.min")?);
    }
    Ok(vec![float_to_value(best)])
}
fn math_clamp(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let x = arg_num(&a, 0, "math.clamp")?;
    let lo = arg_num(&a, 1, "math.clamp")?;
    let hi = arg_num(&a, 2, "math.clamp")?;
    Ok(vec![float_to_value(x.max(lo).min(hi))])
}
fn math_sign(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let x = arg_num(&a, 0, "math.sign")?;
    Ok(vec![Value::Int(if x > 0.0 { 1 } else if x < 0.0 { -1 } else { 0 })])
}
fn math_deg(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.deg")?.to_degrees())])
}
fn math_rad(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Float(arg_num(&a, 0, "math.rad")?.to_radians())])
}
fn math_random(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let f = next_rand_f64();
    match a.len() {
        0 => Ok(vec![Value::Float(f)]),
        1 => {
            let m = arg_num(&a, 0, "math.random")? as i64;
            if m < 1 {
                return Err("math.random: interval is empty".into());
            }
            Ok(vec![Value::Int(1 + (f * m as f64) as i64)])
        }
        _ => {
            let lo = arg_num(&a, 0, "math.random")? as i64;
            let hi = arg_num(&a, 1, "math.random")? as i64;
            if hi < lo {
                return Err("math.random: interval is empty".into());
            }
            Ok(vec![Value::Int(lo + (f * (hi - lo + 1) as f64) as i64)])
        }
    }
}
fn math_randomseed(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let seed = arg_num(&a, 0, "math.randomseed").unwrap_or(0.0) as u64;
    RNG_STATE.with(|r| r.set((seed ^ 0x2545F491_4F6CDD1D) | 1));
    Ok(vec![])
}

fn arg_str(args: &[Value], i: usize, who: &str) -> std::result::Result<String, String> {
    match args.get(i) {
        Some(Value::Str(s)) => Ok(s.to_string()),
        Some(Value::Int(n)) => Ok(n.to_string()),
        Some(Value::Float(x)) => Ok(x.to_string()),
        _ => Err(format!("{who}: expected a string")),
    }
}
fn arg_int(args: &[Value], i: usize, who: &str) -> std::result::Result<i64, String> {
    args.get(i).and_then(coerce_number).map(|n| n.as_f64() as i64).ok_or_else(|| format!("{who}: expected a number"))
}
fn opt_int(args: &[Value], i: usize) -> Option<i64> {
    args.get(i).and_then(coerce_number).map(|n| n.as_f64() as i64)
}
fn arg_tbl(args: &[Value], i: usize, who: &str) -> std::result::Result<Rc<RefCell<Table>>, String> {
    match args.get(i) {
        Some(Value::Table(t)) => Ok(t.clone()),
        _ => Err(format!("{who}: expected a table")),
    }
}

fn str_len(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Int(arg_str(&a, 0, "string.len")?.chars().count() as i64)])
}
fn str_upper(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::str(arg_str(&a, 0, "string.upper")?.to_uppercase())])
}
fn str_lower(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::str(arg_str(&a, 0, "string.lower")?.to_lowercase())])
}
fn str_reverse(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::str(arg_str(&a, 0, "string.reverse")?.chars().rev().collect::<String>())])
}
fn str_rep(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let s = arg_str(&a, 0, "string.rep")?;
    let n = arg_int(&a, 1, "string.rep")?.max(0) as usize;
    let out = match a.get(2).and_then(|v| v.as_str()) {
        Some(sep) if n > 0 => vec![s; n].join(sep),
        _ => s.repeat(n),
    };
    Ok(vec![Value::str(out)])
}
fn str_sub(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let s = arg_str(&a, 0, "string.sub")?;
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    let mut i = opt_int(&a, 1).unwrap_or(1);
    let mut j = opt_int(&a, 2).unwrap_or(-1);
    if i < 0 { i = (len + i + 1).max(1); } else if i == 0 { i = 1; }
    if j < 0 { j = len + j + 1; } else if j > len { j = len; }
    if i > j || i > len {
        return Ok(vec![Value::str("")]);
    }
    let slice: String = chars[(i - 1) as usize..j as usize].iter().collect();
    Ok(vec![Value::str(slice)])
}
fn str_byte(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let s = arg_str(&a, 0, "string.byte")?;
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    let mut i = opt_int(&a, 1).unwrap_or(1);
    if i < 0 { i = len + i + 1; }
    let j = opt_int(&a, 2).unwrap_or(i);
    let mut out = Vec::new();
    let mut k = i.max(1);
    while k <= j.min(len) {
        out.push(Value::Int(chars[(k - 1) as usize] as i64));
        k += 1;
    }
    Ok(out)
}
fn str_char(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let mut s = String::new();
    for idx in 0..a.len() {
        let code = arg_int(&a, idx, "string.char")?;
        match char::from_u32(code as u32) {
            Some(ch) => s.push(ch),
            None => return Err(format!("string.char: invalid code {code}")),
        }
    }
    Ok(vec![Value::str(s)])
}
fn str_find(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {

    let s = arg_str(&a, 0, "string.find")?;
    let pat = arg_str(&a, 1, "string.find")?;
    let init = opt_int(&a, 2).unwrap_or(1).max(1) as usize;
    let start_byte: usize = s.chars().take(init - 1).map(|c| c.len_utf8()).sum();
    if start_byte > s.len() {
        return Ok(vec![Value::Nil]);
    }
    match s[start_byte..].find(&pat) {
        Some(pos) => {
            let bytepos = start_byte + pos;
            let start_char = s[..bytepos].chars().count() + 1;
            let end_char = start_char + pat.chars().count() - 1;
            Ok(vec![Value::Int(start_char as i64), Value::Int(end_char as i64)])
        }
        None => Ok(vec![Value::Nil]),
    }
}
fn str_contains(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Bool(arg_str(&a, 0, "string.contains")?.contains(&arg_str(&a, 1, "string.contains")?))])
}
fn str_startswith(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Bool(arg_str(&a, 0, "string.startswith")?.starts_with(&arg_str(&a, 1, "string.startswith")?))])
}
fn str_endswith(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Bool(arg_str(&a, 0, "string.endswith")?.ends_with(&arg_str(&a, 1, "string.endswith")?))])
}
fn str_trim(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::str(arg_str(&a, 0, "string.trim")?.trim().to_string())])
}
fn str_split(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let s = arg_str(&a, 0, "string.split")?;
    let sep = arg_str(&a, 1, "string.split").unwrap_or_else(|_| " ".to_string());
    let t = Value::table();
    if let Value::Table(rc) = &t {
        if sep.is_empty() {
            for ch in s.chars() {
                rc.borrow_mut().array.push(Value::str(ch.to_string()));
            }
        } else {
            for part in s.split(sep.as_str()) {
                rc.borrow_mut().array.push(Value::str(part.to_string()));
            }
        }
    }
    Ok(vec![t])
}
fn str_format(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let fmt = arg_str(&a, 0, "string.format")?;
    let mut out = String::new();
    let mut argi = 1;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut spec = String::from("%");
        while let Some(&n) = chars.peek() {
            spec.push(n);
            chars.next();
            if n.is_ascii_alphabetic() || n == '%' {
                break;
            }
        }
        match spec.chars().last().unwrap_or('%') {
            '%' => out.push('%'),
            's' => {
                out.push_str(&a.get(argi).map(|v| v.to_string()).unwrap_or_default());
                argi += 1;
            }
            'd' | 'i' => {
                out.push_str(&arg_int(&a, argi, "string.format")?.to_string());
                argi += 1;
            }
            'x' => {
                out.push_str(&format!("{:x}", arg_int(&a, argi, "string.format")?));
                argi += 1;
            }
            'X' => {
                out.push_str(&format!("{:X}", arg_int(&a, argi, "string.format")?));
                argi += 1;
            }
            'f' | 'g' => {
                let n = arg_num(&a, argi, "string.format")?;
                if let Some(dot) = spec.find('.') {
                    let prec: usize = spec[dot + 1..spec.len() - 1].parse().unwrap_or(6);
                    out.push_str(&format!("{n:.prec$}"));
                } else {
                    out.push_str(&n.to_string());
                }
                argi += 1;
            }
            _ => out.push_str(&spec),
        }
    }
    Ok(vec![Value::str(out)])
}

fn tbl_insert(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let t = arg_tbl(&a, 0, "table.insert")?;
    if a.len() >= 3 {
        let pos = arg_int(&a, 1, "table.insert")?.max(1) as usize;
        let v = a[2].clone();
        let mut tb = t.borrow_mut();
        let idx = (pos - 1).min(tb.array.len());
        tb.array.insert(idx, v);
    } else {
        t.borrow_mut().array.push(a.get(1).cloned().unwrap_or(Value::Nil));
    }
    Ok(vec![])
}
fn tbl_remove(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let t = arg_tbl(&a, 0, "table.remove")?;
    let mut tb = t.borrow_mut();
    if tb.array.is_empty() {
        return Ok(vec![Value::Nil]);
    }
    let pos = opt_int(&a, 1).unwrap_or(tb.array.len() as i64);
    if pos < 1 || pos as usize > tb.array.len() {
        return Ok(vec![Value::Nil]);
    }
    Ok(vec![tb.array.remove(pos as usize - 1)])
}
fn tbl_concat(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let t = arg_tbl(&a, 0, "table.concat")?;
    let sep = arg_str(&a, 1, "table.concat").unwrap_or_default();
    let tb = t.borrow();
    let i = opt_int(&a, 2).unwrap_or(1).max(1) as usize;
    let j = opt_int(&a, 3).map(|x| x as usize).unwrap_or(tb.array.len());
    let mut out = String::new();
    for k in i..=j {
        if k >= 1 && k <= tb.array.len() {
            if k > i {
                out.push_str(&sep);
            }
            out.push_str(&tb.array[k - 1].to_string());
        }
    }
    Ok(vec![Value::str(out)])
}
fn tbl_unpack(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let t = arg_tbl(&a, 0, "table.unpack")?;
    let tb = t.borrow();
    let i = opt_int(&a, 1).unwrap_or(1).max(1) as usize;
    let j = opt_int(&a, 2).map(|x| x as usize).unwrap_or(tb.array.len());
    let mut out = Vec::new();
    for k in i..=j {
        if k >= 1 && k <= tb.array.len() {
            out.push(tb.array[k - 1].clone());
        }
    }
    Ok(out)
}
fn tbl_pack(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let t = Value::table();
    if let Value::Table(rc) = &t {
        for v in &a {
            rc.borrow_mut().array.push(v.clone());
        }
        let _ = rc.borrow_mut().set(Value::str("n"), Value::Int(a.len() as i64));
    }
    Ok(vec![t])
}
fn tbl_keys(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let t = arg_tbl(&a, 0, "table.keys")?;
    let tb = t.borrow();
    let out = Value::table();
    if let Value::Table(rc) = &out {
        let mut arr = rc.borrow_mut();
        for idx in 1..=tb.array.len() {
            arr.array.push(Value::Int(idx as i64));
        }
        for k in tb.map.keys() {
            arr.array.push(match k {
                Key::Int(i) => Value::Int(*i),
                Key::Str(s) => Value::str(s.as_str()),
                Key::Bool(b) => Value::Bool(*b),
            });
        }
    }
    Ok(vec![out])
}
fn tbl_sort(interp: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let t = arg_tbl(&a, 0, "table.sort")?;
    let comp = a.get(1).filter(|v| !matches!(v, Value::Nil)).cloned();
    let mut items = t.borrow().array.clone();

    for i in 1..items.len() {
        let mut j = i;
        while j > 0 {
            let less = match &comp {
                Some(c) => interp
                    .call(c, vec![items[j].clone(), items[j - 1].clone()])
                    .map_err(|e| e.0)?
                    .into_iter()
                    .next()
                    .is_some_and(|v| v.is_truthy()),
                None => compare(BinOp::Lt, &items[j], &items[j - 1]).map(|v| v.is_truthy()).unwrap_or(false),
            };
            if less {
                items.swap(j, j - 1);
                j -= 1;
            } else {
                break;
            }
        }
    }
    t.borrow_mut().array = items;
    Ok(vec![])
}

fn bit_band(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let mut r: u32 = 0xFFFF_FFFF;
    for i in 0..a.len() {
        r &= arg_int(&a, i, "bit32.band")? as u32;
    }
    Ok(vec![Value::Int(r as i64)])
}
fn bit_bor(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let mut r: u32 = 0;
    for i in 0..a.len() {
        r |= arg_int(&a, i, "bit32.bor")? as u32;
    }
    Ok(vec![Value::Int(r as i64)])
}
fn bit_bxor(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let mut r: u32 = 0;
    for i in 0..a.len() {
        r ^= arg_int(&a, i, "bit32.bxor")? as u32;
    }
    Ok(vec![Value::Int(r as i64)])
}
fn bit_bnot(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    Ok(vec![Value::Int((!(arg_int(&a, 0, "bit32.bnot")? as u32)) as i64)])
}
fn bit_lshift(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let x = arg_int(&a, 0, "bit32.lshift")? as u32;
    let n = arg_int(&a, 1, "bit32.lshift")?;
    Ok(vec![Value::Int(if !(0..32).contains(&n) { 0 } else { (x << n) as i64 })])
}
fn bit_rshift(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let x = arg_int(&a, 0, "bit32.rshift")? as u32;
    let n = arg_int(&a, 1, "bit32.rshift")?;
    Ok(vec![Value::Int(if !(0..32).contains(&n) { 0 } else { (x >> n) as i64 })])
}
fn bit_arshift(_i: &mut Interpreter, a: Vec<Value>) -> NativeResult {
    let x = arg_int(&a, 0, "bit32.arshift")? as i32;
    let n = arg_int(&a, 1, "bit32.arshift")?;
    let r = if n <= -32 || n >= 32 {
        if x < 0 { -1i32 } else { 0 }
    } else if n >= 0 {
        x >> n
    } else {
        x << (-n)
    };
    Ok(vec![Value::Int((r as u32) as i64)])
}

fn os_time(_i: &mut Interpreter, _a: Vec<Value>) -> NativeResult {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(vec![Value::Int(secs)])
}
fn os_clock(_i: &mut Interpreter, _a: Vec<Value>) -> NativeResult {
    thread_local! {
        static START: std::time::Instant = std::time::Instant::now();
    }
    Ok(vec![Value::Float(START.with(|s| s.elapsed().as_secs_f64()))])
}

fn coro_create(interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let func = args.into_iter().next().unwrap_or(Value::Nil);
    if !matches!(func, Value::Function(_) | Value::Native(_)) {
        return Err(format!("coroutine.create: expected a function, got {}", func.type_name()));
    }
    let global = interp.env.global_scope();
    let state = super::coroutine::create(func, global);
    Ok(vec![Value::Coroutine(Rc::new(RefCell::new(state)))])
}

fn coro_resume(_interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let mut it = args.into_iter();
    let co = it.next().unwrap_or(Value::Nil);
    let rest: Vec<Value> = it.collect();
    match co {
        Value::Coroutine(rc) => Ok(super::coroutine::resume(&rc, rest)),
        other => Err(format!("coroutine.resume: expected a thread, got {}", other.type_name())),
    }
}

fn coro_yield(_interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    super::coroutine::do_yield(args)
}

fn coro_status(_interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match args.first() {
        Some(Value::Coroutine(rc)) => Ok(vec![Value::str(rc.borrow().status_str())]),
        _ => Err("coroutine.status: expected a thread".into()),
    }
}

fn coro_close(_interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match args.first() {
        Some(Value::Coroutine(rc)) => Ok(vec![Value::Bool(super::coroutine::close(rc))]),
        _ => Err("coroutine.close: expected a thread".into()),
    }
}

type NativeResult = std::result::Result<Vec<Value>, String>;

fn builtin_setmetatable(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let table = match args.first() {
        Some(Value::Table(t)) => t.clone(),
        _ => return Err("setmetatable: first argument must be a table".into()),
    };
    match args.get(1) {
        Some(Value::Table(m)) => table.borrow_mut().meta = Some(m.clone()),
        Some(Value::Nil) | None => table.borrow_mut().meta = None,
        Some(other) => {
            return Err(format!(
                "setmetatable: metatable must be a table or nil, got {}",
                other.type_name()
            ));
        }
    }
    Ok(vec![Value::Table(table)])
}

fn builtin_getmetatable(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match args.first() {
        Some(Value::Table(t)) => {
            Ok(vec![t.borrow().meta.clone().map(Value::Table).unwrap_or(Value::Nil)])
        }
        _ => Ok(vec![Value::Nil]),
    }
}

fn builtin_type(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    Ok(vec![Value::str(args.first().map(|v| v.type_name()).unwrap_or("nil"))])
}

fn builtin_tostring(i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let v = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![Value::str(i.display_string(&v).map_err(|e| e.0)?.as_str())])
}

fn builtin_tonumber(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let v = args.first().cloned().unwrap_or(Value::Nil);
    let out = match &v {
        Value::Int(_) | Value::Float(_) => v.clone(),
        Value::Str(s) => {
            let t = s.trim();
            if let Ok(n) = t.parse::<i64>() {
                Value::Int(n)
            } else if let Ok(f) = t.parse::<f64>() {
                Value::Float(f)
            } else {
                Value::Nil
            }
        }
        _ => Value::Nil,
    };
    Ok(vec![out])
}

fn builtin_print(i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let mut parts = Vec::with_capacity(args.len());
    for v in &args {
        parts.push(i.display_string(v).map_err(|e| e.0)?);
    }
    println!("{}", parts.join("\t"));
    Ok(vec![])
}

fn builtin_rawget(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match (args.first(), args.get(1)) {
        (Some(Value::Table(t)), Some(k)) => Ok(vec![t.borrow().get(k)]),
        _ => Err("rawget: expected (table, key)".into()),
    }
}

fn builtin_instanceof(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    let Some(ic) = instance_class(&value) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let result = match args.get(1) {
        Some(Value::Class(c)) => ic.descends_from(c),
        Some(Value::Interface(i)) => ic.implements_interface(i),
        _ => return Err("instanceof: second argument must be a class or interface".into()),
    };
    Ok(vec![Value::Bool(result)])
}

fn builtin_classname(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let value = args.first().cloned().unwrap_or(Value::Nil);

    Ok(vec![as_class(&value).map_or(Value::Nil, |c| Value::str(c.name.as_str()))])
}

fn builtin_classof(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let value = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![instance_class(&value).map_or(Value::Nil, Value::Class)])
}

pub struct NativeClassBuilder {
    name: String,
    methods: HashMap<String, Value>,
    operators: HashMap<String, Value>,
    getters: HashMap<String, Value>,
    setters: HashMap<String, Value>,
    fields: Vec<FieldDef>,
    constructor: Option<Value>,
    is_final: bool,
    is_abstract: bool,
}

impl NativeClassBuilder {

    pub fn new(name: impl Into<String>) -> Self {
        NativeClassBuilder {
            name: name.into(),
            methods: HashMap::new(),
            operators: HashMap::new(),
            getters: HashMap::new(),
            setters: HashMap::new(),
            fields: Vec::new(),
            constructor: None,
            is_final: false,
            is_abstract: false,
        }
    }

    pub fn method(mut self, name: &'static str, func: NativeFn) -> Self {
        self.methods.insert(name.to_string(), Value::native(name, func));
        self
    }

    pub fn getter(mut self, name: &'static str, func: NativeFn) -> Self {
        self.getters.insert(name.to_string(), Value::native(name, func));
        self
    }

    pub fn setter(mut self, name: &'static str, func: NativeFn) -> Self {
        self.setters.insert(name.to_string(), Value::native(name, func));
        self
    }

    pub fn metamethod(mut self, name: &'static str, func: NativeFn) -> Self {
        self.operators.insert(name.to_string(), Value::native(name, func));
        self
    }

    pub fn field(mut self, name: impl Into<String>, default: Value) -> Self {
        self.fields.push(FieldDef { name: name.into(), default: value_to_default_expr(&default) });
        self
    }

    pub fn constructor(mut self, func: NativeFn) -> Self {
        self.constructor = Some(Value::native("constructor", func));
        self
    }

    pub fn make_final(mut self) -> Self {
        self.is_final = true;
        self
    }

    pub fn make_abstract(mut self) -> Self {
        self.is_abstract = true;
        self
    }

    pub fn build(self) -> Value {
        let instance_meta = Value::table();
        let Value::Table(meta_rc) = &instance_meta else { unreachable!() };
        let statics = Value::table();
        let Value::Table(statics_rc) = &statics else { unreachable!() };
        let class = Rc::new(Class {
            name: self.name,
            parent: None,
            methods: self.methods,
            operators: self.operators,
            constructor: self.constructor,
            fields: self.fields,
            statics: statics_rc.clone(),
            getters: self.getters,
            setters: self.setters,
            access: HashMap::new(),
            abstracts: HashSet::new(),
            finals: HashSet::new(),
            is_final: self.is_final,
            is_abstract: self.is_abstract,
            interfaces: Vec::new(),
            instance_meta: meta_rc.clone(),
            gc_mark: std::cell::Cell::new(false),
        });
        gc::register_class(&class);
        meta_rc.borrow_mut().set(Value::str("__class"), Value::Class(class.clone())).ok();
        Value::Class(class)
    }
}

fn value_to_default_expr(v: &Value) -> Option<Expr> {
    Some(match v {
        Value::Bool(b) => Expr::Bool(*b),
        Value::Int(n) => Expr::Int(*n),
        Value::Float(f) => Expr::Float(*f),
        Value::Str(s) => Expr::Str(s.to_string()),
        _ => return None,
    })
}

fn as_class(v: &Value) -> Option<Rc<Class>> {
    match v {
        Value::Class(c) => Some(c.clone()),
        _ => instance_class(v),
    }
}

fn is_behaviour(class: &Rc<Class>) -> bool {
    let mut cur = class.parent.clone();
    while let Some(c) = cur {
        if c.name == "MonoBehaviour" {
            return true;
        }
        cur = c.parent.clone();
    }
    false
}

fn builtin_run(i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let frames = match args.first() {
        Some(Value::Int(n)) => (*n).max(0),
        Some(Value::Float(f)) => (*f as i64).max(0),
        _ => 1,
    };
    let classes = i.behaviours.clone();
    let mut instances = Vec::with_capacity(classes.len());
    for c in classes {
        let inst = i.construct(c, Vec::new()).map_err(|e| e.0)?.into_iter().next().unwrap_or(Value::Nil);
        i.call_lifecycle(&inst, "Awake").map_err(|e| e.0)?;
        i.call_lifecycle(&inst, "Start").map_err(|e| e.0)?;
        instances.push(inst);
    }
    for _ in 0..frames {
        for inst in &instances {
            i.call_lifecycle(inst, "Update").map_err(|e| e.0)?;
        }
    }
    for inst in &instances {
        i.call_lifecycle(inst, "OnDestroy").map_err(|e| e.0)?;
    }
    Ok(vec![])
}

fn builtin_spawn(i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let mut it = args.into_iter();
    let Some(Value::Class(c)) = it.next() else {
        return Err("spawn expects a class as its first argument".into());
    };
    let rest: Vec<Value> = it.collect();
    let inst = i.construct(c, rest).map_err(|e| e.0)?.into_iter().next().unwrap_or(Value::Nil);
    i.call_lifecycle(&inst, "Awake").map_err(|e| e.0)?;
    i.call_lifecycle(&inst, "Start").map_err(|e| e.0)?;
    Ok(vec![inst])
}

fn builtin_superclass(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let v = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![as_class(&v).and_then(|c| c.parent.clone()).map_or(Value::Nil, Value::Class)])
}

fn builtin_isabstract(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let v = args.first().cloned().unwrap_or(Value::Nil);
    Ok(vec![Value::Bool(as_class(&v).is_some_and(|c| c.is_abstract))])
}

fn builtin_methodsof(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let v = args.first().cloned().unwrap_or(Value::Nil);
    let out = Value::table();
    if let (Some(class), Value::Table(rc)) = (as_class(&v), &out) {
        let mut seen = std::collections::HashSet::new();
        let mut cur = Some(class);
        while let Some(c) = cur {
            for name in c.methods.keys() {
                if seen.insert(name.clone()) {
                    rc.borrow_mut().array.push(Value::str(name.as_str()));
                }
            }
            cur = c.parent.clone();
        }
    }
    Ok(vec![out])
}

fn builtin_rawequal(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let a = args.first().cloned().unwrap_or(Value::Nil);
    let b = args.get(1).cloned().unwrap_or(Value::Nil);
    Ok(vec![Value::Bool(values_equal(&a, &b))])
}

fn builtin_rawlen(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match args.first() {
        Some(Value::Table(t)) => Ok(vec![Value::Int(t.borrow().len() as i64)]),
        Some(Value::Str(s)) => Ok(vec![Value::Int(s.chars().count() as i64)]),
        _ => Err("rawlen: expected a table or string".into()),
    }
}

thread_local! {

    static MODULE_CACHE: RefCell<std::collections::HashMap<String, Value>> =
        RefCell::new(std::collections::HashMap::new());
}

fn builtin_require(interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    use std::path::PathBuf;
    let raw = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| "require: expected a string path".to_string())?
        .to_string();

    let dir = interp.module_dir.clone();

    if raw == "@self" {
        return Ok(vec![folder_children_table(interp, &dir)?]);
    }

    let base: PathBuf = if let Some(rest) = raw.strip_prefix('@') {
        let (alias, tail) = match rest.split_once('/') {
            Some((a, t)) => (a, Some(t)),
            None => (rest, None),
        };
        let target = luarrc_alias(&dir, alias).ok_or_else(|| {
            format!("require: unknown alias '@{alias}' (define it in a .luarrc file)")
        })?;
        match tail {
            Some(t) => target.join(t),
            None => target,
        }
    } else {
        dir.join(&raw)
    };

    let file = resolve_module_file(&base)
        .ok_or_else(|| format!("require: cannot find module '{raw}'"))?;

    let key = std::fs::canonicalize(&file)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| file.to_string_lossy().into_owned());

    if let Some(cached) = MODULE_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(vec![cached]);
    }

    let source = std::fs::read_to_string(&file)
        .map_err(|e| format!("require: cannot read '{}': {e}", file.display()))?;
    let tokens = crate::lexer::tokenize(&source).map_err(|e| e.to_string())?;
    let program = crate::parser::parse(tokens).map_err(|e| e.to_string())?;

    let global = interp.env.global_scope();
    let mut module = Interpreter::with_shared_global(global);
    module.module_dir = file.parent().map(PathBuf::from).unwrap_or(dir);
    module.env.push_scope();
    module.env.mark_module_root();
    let returned = module.run(&program).map_err(|e| e.0)?;
    let value = returned.into_iter().next().unwrap_or(Value::Nil);

    MODULE_CACHE.with(|c| c.borrow_mut().insert(key, value.clone()));
    Ok(vec![value])
}

fn resolve_module_file(base: &std::path::Path) -> Option<std::path::PathBuf> {
    let direct = base.with_extension("luar");
    if direct.is_file() {
        return Some(direct);
    }
    let init = base.join("init.luar");
    if init.is_file() {
        return Some(init);
    }
    None
}

fn luarrc_alias(dir: &std::path::Path, alias: &str) -> Option<std::path::PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let rc = d.join(".luarrc");
        if let Ok(text) = std::fs::read_to_string(&rc) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((name, target)) = line.split_once('=') {
                    let name = name.trim().trim_start_matches('@');
                    if name == alias {
                        return Some(d.join(target.trim().trim_matches('"')));
                    }
                }
            }
        }
        cur = d.parent();
    }
    None
}

fn folder_children_table(interp: &mut Interpreter, dir: &std::path::Path) -> std::result::Result<Value, String> {
    let table = Value::table();
    let Value::Table(rc) = &table else { unreachable!() };
    let mut names: Vec<(String, std::path::PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "luar").unwrap_or(false) {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if stem != "init" {
                    names.push((stem, path));
                }
            } else if path.is_dir() && path.join("init.luar").is_file() {
                let stem = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                names.push((stem, path));
            }
        }
    }
    names.sort();
    for (name, path) in names {
        let value = require_file(interp, &path)?;
        rc.borrow_mut().set(Value::str(name.as_str()), value)?;
    }
    Ok(table)
}

fn require_file(interp: &mut Interpreter, path: &std::path::Path) -> std::result::Result<Value, String> {
    use std::path::PathBuf;
    let file = if path.is_dir() { path.join("init.luar") } else { path.with_extension("luar") };
    let key = std::fs::canonicalize(&file)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| file.to_string_lossy().into_owned());
    if let Some(cached) = MODULE_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(cached);
    }
    let source = std::fs::read_to_string(&file)
        .map_err(|e| format!("require: cannot read '{}': {e}", file.display()))?;
    let tokens = crate::lexer::tokenize(&source).map_err(|e| e.to_string())?;
    let program = crate::parser::parse(tokens).map_err(|e| e.to_string())?;
    let global = interp.env.global_scope();
    let mut module = Interpreter::with_shared_global(global);
    module.module_dir = file.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    module.env.push_scope();
    module.env.mark_module_root();
    let returned = module.run(&program).map_err(|e| e.0)?;
    let value = returned.into_iter().next().unwrap_or(Value::Nil);
    MODULE_CACHE.with(|c| c.borrow_mut().insert(key, value.clone()));
    Ok(value)
}

fn builtin_rawset(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match (args.first(), args.get(1), args.get(2)) {
        (Some(Value::Table(t)), Some(k), Some(v)) => {
            t.borrow_mut().set(k.clone(), v.clone())?;
            Ok(vec![Value::Table(t.clone())])
        }
        _ => Err("rawset: expected (table, key, value)".into()),
    }
}

fn builtin_collectgarbage(_i: &mut Interpreter, _args: Vec<Value>) -> NativeResult {
    gc::request();
    Ok(vec![])
}

fn builtin_pcall(interp: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let mut it = args.into_iter();
    let Some(callee) = it.next() else {
        return Err("pcall: missing function argument".into());
    };
    let call_args: Vec<Value> = it.collect();
    match interp.call(&callee, call_args) {
        Ok(mut results) => {
            let mut out = vec![Value::Bool(true)];
            out.append(&mut results);
            Ok(out)
        }
        Err(e) => Ok(vec![Value::Bool(false), Value::str(e.0)]),
    }
}

fn builtin_ipairs(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match args.first() {
        Some(Value::Table(t)) => Ok(vec![
            Value::Native(Native { name: "ipairs_iter", func: ipairs_iter }),
            Value::Table(t.clone()),
            Value::Int(0),
        ]),
        _ => Err("ipairs: expected a table".into()),
    }
}

fn ipairs_iter(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let table = match args.first() {
        Some(Value::Table(t)) => t.clone(),
        _ => return Err("ipairs iterator: expected a table".into()),
    };
    let i = match args.get(1) {
        Some(Value::Int(i)) => *i,
        Some(Value::Float(f)) => *f as i64,
        _ => 0,
    };
    let next_index = i + 1;
    let value = table.borrow().get(&Value::Int(next_index));
    if matches!(value, Value::Nil) {
        Ok(vec![Value::Nil])
    } else {
        Ok(vec![Value::Int(next_index), value])
    }
}

fn builtin_pairs(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    match args.first() {
        Some(Value::Table(t)) => Ok(vec![
            Value::Native(Native { name: "next", func: builtin_next }),
            Value::Table(t.clone()),
            Value::Nil,
        ]),
        _ => Err("pairs: expected a table".into()),
    }
}

fn builtin_next(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
    let table = match args.first() {
        Some(Value::Table(t)) => t.clone(),
        _ => return Err("next: expected a table".into()),
    };
    let key = args.get(1).cloned().unwrap_or(Value::Nil);

    let tb = table.borrow();
    let mut entries: Vec<(Value, Value)> = Vec::with_capacity(tb.array.len() + tb.map.len());
    for (idx, v) in tb.array.iter().enumerate() {
        entries.push((Value::Int(idx as i64 + 1), v.clone()));
    }
    for (k, v) in tb.map.iter() {
        let key_val = match k {
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::str(s.as_str()),
            Key::Bool(b) => Value::Bool(*b),
        };
        entries.push((key_val, v.clone()));
    }

    let start = if matches!(key, Value::Nil) {
        0
    } else {
        match entries.iter().position(|(k, _)| values_equal(k, &key)) {
            Some(i) => i + 1,
            None => return Ok(vec![Value::Nil]),
        }
    };
    match entries.get(start) {
        Some((k, v)) => Ok(vec![k.clone(), v.clone()]),
        None => Ok(vec![Value::Nil]),
    }
}

fn compound_binop(op: AssignOp) -> BinOp {
    match op {
        AssignOp::Add => BinOp::Add,
        AssignOp::Sub => BinOp::Sub,
        AssignOp::Mul => BinOp::Mul,
        AssignOp::Div => BinOp::Div,
        AssignOp::Mod => BinOp::Mod,
        AssignOp::Concat => BinOp::Concat,
        AssignOp::Assign => unreachable!("plain assignment has no binary op"),
    }
}

fn eval_unary(op: UnaryOp, v: Value) -> Result<Value> {
    match op {
        UnaryOp::Not => Ok(Value::Bool(!v.is_truthy())),
        UnaryOp::Neg => match coerce_number(&v) {
            Some(Num::Int(i)) => Ok(Value::Int(-i)),
            Some(Num::Float(x)) => Ok(Value::Float(-x)),
            None => Err(EvalError(format!("cannot negate a {}", v.type_name()))),
        },
        UnaryOp::Len => match &v {
            Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
            Value::Table(t) => Ok(Value::Int(t.borrow().len() as i64)),
            other => Err(EvalError(format!("cannot take length of a {}", other.type_name()))),
        },
    }
}

#[derive(Clone, Copy)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(x) => x,
        }
    }
}

fn coerce_number(v: &Value) -> Option<Num> {
    match v {
        Value::Int(i) => Some(Num::Int(*i)),
        Value::Float(x) => Some(Num::Float(*x)),
        Value::Str(s) => {
            let t = s.trim();
            if let Ok(i) = t.parse::<i64>() {
                Some(Num::Int(i))
            } else {
                t.parse::<f64>().ok().map(Num::Float)
            }
        }
        _ => None,
    }
}

fn coerce_concat(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        Value::Int(i) => Some(i.to_string()),
        Value::Float(x) => Some(x.to_string()),
        _ => None,
    }
}

fn apply_binop(op: BinOp, a: Value, b: Value) -> Result<Value> {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div | Mod | Pow => arithmetic(op, &a, &b),
        Concat => {
            let (x, y) = (coerce_concat(&a), coerce_concat(&b));
            match (x, y) {
                (Some(x), Some(y)) => Ok(Value::str(format!("{x}{y}"))),
                _ => Err(EvalError(format!(
                    "cannot concatenate {} and {}",
                    a.type_name(),
                    b.type_name()
                ))),
            }
        }
        Eq => Ok(Value::Bool(values_equal(&a, &b))),
        Ne => Ok(Value::Bool(!values_equal(&a, &b))),
        Lt | Le | Gt | Ge => compare(op, &a, &b),
    }
}

fn arithmetic(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    use BinOp::*;
    let (x, y) = match (coerce_number(a), coerce_number(b)) {
        (Some(x), Some(y)) => (x, y),
        _ => {
            return Err(EvalError(format!(
                "cannot perform arithmetic on {} and {}",
                a.type_name(),
                b.type_name()
            )));
        }
    };

    if let (Num::Int(xi), Num::Int(yi)) = (x, y) {
        match op {
            Add => return Ok(Value::Int(xi.wrapping_add(yi))),
            Sub => return Ok(Value::Int(xi.wrapping_sub(yi))),
            Mul => return Ok(Value::Int(xi.wrapping_mul(yi))),
            Mod => {
                if yi == 0 {
                    return Err(EvalError("modulo by zero".into()));
                }
                return Ok(Value::Int(xi.rem_euclid(yi)));
            }
            _ => {}
        }
    }

    let (xf, yf) = (x.as_f64(), y.as_f64());
    let result = match op {
        Add => xf + yf,
        Sub => xf - yf,
        Mul => xf * yf,
        Div => xf / yf,
        Mod => xf - (xf / yf).floor() * yf,
        Pow => xf.powf(yf),
        _ => unreachable!(),
    };
    Ok(Value::Float(result))
}

fn compare(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    use std::cmp::Ordering;
    let ordering = match (a, b) {
        (Value::Str(x), Value::Str(y)) => x.cmp(y),
        _ => {

            let (x, y) = match (a, b) {
                (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => {
                    (number_f64(a), number_f64(b))
                }
                _ => {
                    return Err(EvalError(format!(
                        "cannot compare {} and {}",
                        a.type_name(),
                        b.type_name()
                    )));
                }
            };
            x.partial_cmp(&y)
                .ok_or_else(|| EvalError("comparison with NaN".into()))?
        }
    };
    let result = match op {
        BinOp::Lt => ordering == Ordering::Less,
        BinOp::Le => ordering != Ordering::Greater,
        BinOp::Gt => ordering == Ordering::Greater,
        BinOp::Ge => ordering != Ordering::Less,
        _ => unreachable!(),
    };
    Ok(Value::Bool(result))
}

fn number_f64(v: &Value) -> f64 {
    match v {
        Value::Int(i) => *i as f64,
        Value::Float(x) => *x,
        _ => f64::NAN,
    }
}

fn operator_metamethod(op: BinOp) -> Option<&'static str> {
    use BinOp::*;
    Some(match op {
        Add => "__add",
        Sub => "__sub",
        Mul => "__mul",
        Div => "__div",
        Mod => "__mod",
        Pow => "__pow",
        Concat => "__concat",
        Eq | Ne => "__eq",
        Lt => "__lt",
        Le => "__le",
        Gt | Ge => return None,
    })
}

fn get_metamethod(v: &Value, name: &str) -> Option<Value> {
    match v {
        Value::Table(t) => t.borrow().metamethod(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    fn run(src: &str) -> Interpreter {
        let program = parse(tokenize(src).unwrap()).unwrap();
        let mut interp = Interpreter::new();
        interp.run(&program).unwrap();
        interp
    }

    #[test]
    fn declarations_and_arithmetic() {
        let i = run("pub local x = (1 + 1) + 1\npub local y = x * 2");
        assert_eq!(i.env.get("x"), Some(Value::Int(3)));
        assert_eq!(i.env.get("y"), Some(Value::Int(6)));
    }

    #[test]
    fn const_cannot_be_reassigned() {
        let program = parse(tokenize("const c = 1\nc = 2").unwrap()).unwrap();
        let mut interp = Interpreter::new();
        assert!(interp.run(&program).is_err());
    }

    #[test]
    fn math_with_strings() {

        let i = run(r#"pub local a = "10" + 5
pub local b = "2.5" * 2"#);
        assert_eq!(i.env.get("a"), Some(Value::Int(15)));
        assert_eq!(i.env.get("b"), Some(Value::Float(5.0)));
    }

    #[test]
    fn concatenation() {
        let i = run(r#"pub local s = "v" .. 1 .. "_" .. 2"#);
        assert_eq!(i.env.get("s"), Some(Value::str("v1_2")));
    }

    #[test]
    fn class_construct_fields_methods() {
        let i = run(
            r#"class Point {
                 public x: number = 0
                 public y: number = 0
                 constructor(x, y) self.x = x; self.y = y end
                 function sum() return self.x + self.y end
               }
               const p = Point(3, 4)
               pub local total = p:sum()
               pub local px = p.x"#,
        );
        assert_eq!(i.env.get("total"), Some(Value::Int(7)));
        assert_eq!(i.env.get("px"), Some(Value::Int(3)));
    }

    #[test]
    fn class_inheritance_override_super() {
        let i = run(
            r#"class A {
                 function who() return "A" end
               }
               class B extends A {
                 override function who() return super:who() .. "B" end
               }
               pub local r = B():who()"#,
        );
        assert_eq!(i.env.get("r"), Some(Value::str("AB")));
    }

    #[test]
    fn class_operator_overload() {
        let i = run(
            r#"class Vec {
                 public n: number = 0
                 constructor(n) self.n = n end
                 operator +(o) return Vec(self.n + o.n) end
               }
               pub local sum = (Vec(2) + Vec(3)).n"#,
        );
        assert_eq!(i.env.get("sum"), Some(Value::Int(5)));
    }

    #[test]
    fn varargs_collect() {
        let i = run(
            r#"function count(...) return #({ ... }) end
               pub local n = count(1, 2, 3, 4)"#,
        );
        assert_eq!(i.env.get("n"), Some(Value::Int(4)));
    }

    #[test]
    fn private_access_is_enforced() {

        let program = parse(
            tokenize(
                r#"class Secret { private s: number = 1 }
                   const x = Secret()
                   local v = x.s"#,
            )
            .unwrap(),
        )
        .unwrap();
        let mut interp = Interpreter::new();
        assert!(interp.run(&program).is_err());
    }

    #[test]
    fn instanceof_walks_chain() {
        let i = run(
            r#"class Base {}
               class Derived extends Base {}
               const d = Derived()
               pub local a = instanceof(d, Base)
               pub local b = instanceof(d, Derived)"#,
        );
        assert_eq!(i.env.get("a"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("b"), Some(Value::Bool(true)));
    }

    #[test]
    fn interface_conformance_enforced() {

        let program = parse(
            tokenize("interface Shape { function area() }\nclass Bad implements Shape {}").unwrap(),
        )
        .unwrap();
        let mut interp = Interpreter::new();
        assert!(interp.run(&program).is_err());
    }

    #[test]
    fn final_class_cannot_be_extended() {
        let program =
            parse(tokenize("final class A {}\nclass B extends A {}").unwrap()).unwrap();
        let mut interp = Interpreter::new();
        assert!(interp.run(&program).is_err());
    }

    #[test]
    fn mixin_composes_methods() {
        let i = run(
            r#"class M { function greet() return "hi" end }
               class C mixin M { public x: number = 1 }
               pub local g = C():greet()"#,
        );
        assert_eq!(i.env.get("g"), Some(Value::str("hi")));
    }

    #[test]
    fn abstract_class_cannot_be_instantiated() {
        let program = parse(
            tokenize("abstract class A { abstract function f() end }\nconst a = A()").unwrap(),
        )
        .unwrap();
        let mut interp = Interpreter::new();
        assert!(interp.run(&program).is_err());
    }

    #[test]
    fn override_without_parent_method_errors() {
        let program = parse(
            tokenize("class A {}\nclass B extends A { override function f() end }").unwrap(),
        )
        .unwrap();
        let mut interp = Interpreter::new();
        assert!(interp.run(&program).is_err());
    }

    #[test]
    fn operator_override_in_subclass() {
        let i = run(
            r#"class Base { operator +(o) return 1 end }
               class Sub extends Base { operator +(o) return 99 end }
               pub local r = Sub() + Sub()"#,
        );
        assert_eq!(i.env.get("r"), Some(Value::Int(99)));
    }

    #[test]
    fn tostring_and_tonumber_builtins() {
        let i = run(
            r#"pub local a = tonumber("42")
               pub local b = tonumber("3.5")
               pub local c = tostring(7)
               pub local d = tonumber("nope")"#,
        );
        assert_eq!(i.env.get("a"), Some(Value::Int(42)));
        assert_eq!(i.env.get("b"), Some(Value::Float(3.5)));
        assert_eq!(i.env.get("c"), Some(Value::str("7")));
        assert_eq!(i.env.get("d"), Some(Value::Nil));
    }

    #[test]
    fn reflection_superclass_and_isabstract() {
        let i = run(
            r#"abstract class A { abstract function f() end }
               class B extends A { override function f() end }
               const b = B()
               pub local up = classname(superclass(b))
               pub local ab = isabstract(A)
               pub local cb = isabstract(B)"#,
        );
        assert_eq!(i.env.get("up"), Some(Value::str("A")));
        assert_eq!(i.env.get("ab"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("cb"), Some(Value::Bool(false)));
    }

    #[test]
    fn native_class_defined_from_rust() {
        fn greet(_i: &mut Interpreter, _args: Vec<Value>) -> NativeResult {
            Ok(vec![Value::str("hi from rust")])
        }
        fn bump(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {
            let this = &args[0];
            let cur = this.field(&Value::str("count"));
            let next = Value::Int(cur.as_int().unwrap_or(0) + 1);
            this.set_field(Value::str("count"), next.clone())?;
            Ok(vec![next])
        }
        fn init(_i: &mut Interpreter, args: Vec<Value>) -> NativeResult {

            let start = args.get(1).cloned().unwrap_or(Value::Int(0));
            args[0].set_field(Value::str("count"), start)?;
            Ok(vec![])
        }
        let mut interp = Interpreter::new();
        interp.define_class(
            NativeClassBuilder::new("Counter")
                .field("count", Value::Int(0))
                .constructor(init)
                .method("greet", greet)
                .method("bump", bump),
        );
        interp
            .run_source(
                r#"const c = Counter(10)
                   pub local g = c:greet()
                   c:bump()
                   pub local n = c:bump()"#,
            )
            .unwrap();
        assert_eq!(interp.env.get("g"), Some(Value::str("hi from rust")));
        assert_eq!(interp.env.get("n"), Some(Value::Int(12)));
    }

    #[test]
    fn monobehaviour_run_drives_lifecycle() {
        let i = run(
            r#"class Counter extends MonoBehaviour {
                 public ticks: number = 0
                 function Start() self.ticks = 100 end
                 function Update() self.ticks += 1 end
               }
               pub local seen = instanceof(Counter(), MonoBehaviour)
               run(3)"#,
        );
        assert_eq!(i.env.get("seen"), Some(Value::Bool(true)));
    }

    #[test]
    fn spawn_runs_start_and_returns_instance() {
        let i = run(
            r#"class Bot extends MonoBehaviour {
                 public ready: boolean = false
                 function Start() self.ready = true end
               }
               const b = spawn(Bot)
               pub local r = b.ready"#,
        );
        assert_eq!(i.env.get("r"), Some(Value::Bool(true)));
    }

    #[test]
    fn enums_auto_increment_and_add_on() {
        let i = run(
            r#"enum Color { Red Green Blue }
               enum Color { Yellow }
               enum Status { Active = 10 Inactive Banned = 99 }
               pub local r = Color.Red
               pub local b = Color.Blue
               pub local y = Color.Yellow
               pub local ina = Status.Inactive
               pub local ban = Status.Banned"#,
        );
        assert_eq!(i.env.get("r"), Some(Value::Int(0)));
        assert_eq!(i.env.get("b"), Some(Value::Int(2)));
        assert_eq!(i.env.get("y"), Some(Value::Int(3)));
        assert_eq!(i.env.get("ina"), Some(Value::Int(11)));
        assert_eq!(i.env.get("ban"), Some(Value::Int(99)));
    }

    #[test]
    fn string_forms() {
        let i = run(
            r#"local name = "x"
               local count = 3
               pub local interp = `hi {name} {count + count}`
               pub local esc = "a\tb\nc\\d"
               pub local long = [[one
two]]
               pub local nested = [==[ has ]] inside ]==]
               pub local brace = `lit \{x\} {name}`"#,
        );
        assert_eq!(i.env.get("interp"), Some(Value::str("hi x 6")));
        assert_eq!(i.env.get("esc"), Some(Value::str("a\tb\nc\\d")));
        assert_eq!(i.env.get("long"), Some(Value::str("one\ntwo")));
        assert_eq!(i.env.get("nested"), Some(Value::str(" has ]] inside ")));
        assert_eq!(i.env.get("brace"), Some(Value::str("lit {x} x")));
    }

    #[test]
    fn enums_are_always_global() {
        let i = run(
            r#"function make() enum Dir { North South } end
               make()
               pub local n = Dir.North"#,
        );
        assert_eq!(i.env.get("n"), Some(Value::Int(0)));
    }

    #[test]
    fn generic_type_params_are_scrubbed() {

        let i = run(
            r#"type Box<T> = { value: T }
               local function id<T>(x) return x end
               class C<T> { function get<U>() return 5 end }
               pub local r = id(C():get())"#,
        );
        assert_eq!(i.env.get("r"), Some(Value::Int(5)));
    }

    #[test]
    fn getters_and_setters() {
        let i = run(
            r#"class Box {
                 private v: number = 0
                 get value() return self.v end
                 set value(x) self.v = x * 2 end
               }
               const b = Box()
               b.value = 5
               pub local got = b.value"#,
        );
        assert_eq!(i.env.get("got"), Some(Value::Int(10)));
    }

    #[test]
    fn math_library_basics() {
        let i = run(
            r#"pub local a = math.sqrt(81)
               pub local b = math.max(3, 9, 5)
               pub local c = math.floor(2.9)
               pub local d = math.abs(-7)
               pub local e = 2 ^ 8
               pub local f = 17 % 5"#,
        );
        assert_eq!(i.env.get("a"), Some(Value::Int(9)));
        assert_eq!(i.env.get("b"), Some(Value::Int(9)));
        assert_eq!(i.env.get("c"), Some(Value::Int(2)));
        assert_eq!(i.env.get("d"), Some(Value::Int(7)));
        assert_eq!(i.env.get("e"), Some(Value::Float(256.0)));
        assert_eq!(i.env.get("f"), Some(Value::Int(2)));
    }

    #[test]
    fn table_call_metamethod() {
        let i = run(
            r#"local d = setmetatable({}, { __call = function(self, x) return x + 1 end })
               pub local r = d(41)"#,
        );
        assert_eq!(i.env.get("r"), Some(Value::Int(42)));
    }

    #[test]
    fn comparisons_make_bools() {
        let i = run("pub local t = (1 + 1) == 2\npub local f = 3 < 2");
        assert_eq!(i.env.get("t"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("f"), Some(Value::Bool(false)));
    }

    #[test]
    fn and_or_idiom_returns_values() {
        let i = run(r#"pub local picked = false and "no" or "yes""#);
        assert_eq!(i.env.get("picked"), Some(Value::str("yes")));
    }

    #[test]
    fn compound_assignment() {
        let i = run("local n = 10\nn += 5\nn -= 2\npub local out = n");
        assert_eq!(i.env.get("out"), Some(Value::Int(13)));
    }

    #[test]
    fn tables_dicts_and_indexing() {
        let i = run(r#"pub local t = {10, 20, ["Test"] = true}
t[2] = 99
pub local first = t[1]
pub local flag = t["Test"]
pub local second = t[2]"#);
        assert_eq!(i.env.get("first"), Some(Value::Int(10)));
        assert_eq!(i.env.get("flag"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("second"), Some(Value::Int(99)));
    }

    #[test]
    fn if_with_logical_conditions() {
        let i = run(r#"pub local r = 0
if 1 < 2 and 3 > 2 then
  r = 1
else
  r = 2
end"#);
        assert_eq!(i.env.get("r"), Some(Value::Int(1)));
    }

    #[test]
    fn scopes_clean_up_locals() {
        let i = run("do\n  local temp = 5\nend");
        assert_eq!(i.env.get("temp"), None);
    }

    #[test]
    fn comments_are_filtered() {
        let i = run("-- a leading comment\npub local x = 1 -- trailing\n--[[ block\nspanning ]]\npub local y = 2");
        assert_eq!(i.env.get("x"), Some(Value::Int(1)));
        assert_eq!(i.env.get("y"), Some(Value::Int(2)));
    }

    #[test]
    fn metatable_index_provides_inheritance() {

        let i = run(
            r#"local base = {greeting = "hi"}
local meta = {__index = base}
pub local obj = {}
setmetatable(obj, meta)
pub local g = obj.greeting
pub local missing = obj.nope"#,
        );
        assert_eq!(i.env.get("g"), Some(Value::str("hi")));
        assert_eq!(i.env.get("missing"), Some(Value::Nil));
    }

    #[test]
    fn getmetatable_and_type_builtins() {
        let i = run(
            r#"pub local t = {}
local m = {}
setmetatable(t, m)
pub local same = getmetatable(t) == m
pub local kind = type(t)"#,
        );
        assert_eq!(i.env.get("same"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("kind"), Some(Value::str("table")));
    }

    #[test]
    fn rawget_bypasses_metatable() {
        let i = run(
            r#"local base = {x = 1}
pub local obj = {}
setmetatable(obj, {__index = base})
pub local viameta = obj.x
pub local raw = rawget(obj, "x")"#,
        );
        assert_eq!(i.env.get("viameta"), Some(Value::Int(1)));
        assert_eq!(i.env.get("raw"), Some(Value::Nil));
    }

    #[test]
    fn while_loop_runs_until_condition_false() {
        let i = run(
            r#"local n = 0
pub local total = 0
while n < 5 do
  total += n
  n += 1
end"#,
        );
        assert_eq!(i.env.get("total"), Some(Value::Int(10)));
    }

    #[test]
    fn break_exits_loop() {
        let i = run(
            r#"pub local last = 0
local i = 1
while true do
  last = i
  if i >= 3 then break end
  i += 1
end"#,
        );
        assert_eq!(i.env.get("last"), Some(Value::Int(3)));
    }

    #[test]
    fn numeric_for_loop() {
        let i = run("pub local sum = 0\nfor k = 1, 4 do sum += k end");
        assert_eq!(i.env.get("sum"), Some(Value::Int(10)));
    }

    #[test]
    fn for_in_ipairs_and_pairs() {
        let i = run(
            r#"local arr = {10, 20, 30}
pub local sum = 0
for index, value in ipairs(arr) do
  sum += value
end
local dict = {a = 1, b = 2, c = 3}
pub local count = 0
for key, value in pairs(dict) do
  count += value
end"#,
        );
        assert_eq!(i.env.get("sum"), Some(Value::Int(60)));
        assert_eq!(i.env.get("count"), Some(Value::Int(6)));
    }

    #[test]
    fn functions_with_return() {
        let i = run(
            r#"local function add(a, b)
  return a + b
end
pub local result = add(3, 4)"#,
        );
        assert_eq!(i.env.get("result"), Some(Value::Int(7)));
    }

    #[test]
    fn functions_are_closures() {
        let i = run(
            r#"local function counter()
  local n = 0
  return function()
    n += 1
    return n
  end
end
local tick = counter()
pub local a = tick()
pub local b = tick()
pub local c = tick()"#,
        );
        assert_eq!(i.env.get("a"), Some(Value::Int(1)));
        assert_eq!(i.env.get("b"), Some(Value::Int(2)));
        assert_eq!(i.env.get("c"), Some(Value::Int(3)));
    }

    #[test]
    fn recursive_function() {
        let i = run(
            r#"local function fact(n)
  if n <= 1 then return 1 end
  return n * fact(n - 1)
end
pub local f5 = fact(5)"#,
        );
        assert_eq!(i.env.get("f5"), Some(Value::Int(120)));
    }

    #[test]
    fn const_function_and_pub() {
        let i = run(
            r#"const function double(x) return x * 2 end
pub local out = double(21)"#,
        );
        assert_eq!(i.env.get("out"), Some(Value::Int(42)));
    }

    #[test]
    fn pcall_catches_errors() {
        let i = run(
            r#"local function boom() return undefined_global end
local okBad, errBad = pcall(boom)
pub local ok_bad = okBad
local function fine() return 42 end
local okGood, value = pcall(fine)
pub local ok_good = okGood
pub local good_value = value"#,
        );
        assert_eq!(i.env.get("ok_bad"), Some(Value::Bool(false)));
        assert_eq!(i.env.get("ok_good"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("good_value"), Some(Value::Int(42)));
    }

    #[test]
    fn function_as_table_value_is_callable() {
        let i = run(
            r#"local obj = {}
obj.greet = function(name) return "hi " .. name end
pub local msg = obj.greet("luar")"#,
        );
        assert_eq!(i.env.get("msg"), Some(Value::str("hi luar")));
    }

    #[test]
    fn bare_declaration_is_immutable_local() {

        let program = parse(tokenize("x = 1\nx = 2").unwrap()).unwrap();
        let mut interp = Interpreter::new();
        assert!(interp.run(&program).is_err());

        let scoped = parse(tokenize("do\n  hidden = 5\nend\npub local seen = hidden").unwrap()).unwrap();
        let mut interp2 = Interpreter::new();
        assert!(interp2.run(&scoped).is_err(), "`hidden` must not escape the do-block");
    }

    #[test]
    fn pub_bare_declaration_is_global() {
        let i = run("pub shared = 42");
        assert_eq!(i.env.get("shared"), Some(Value::Int(42)));
    }

    #[test]
    fn semicolons_separate_statements() {
        let i = run("local a = 1; local b = 2; pub local c = a + b;");
        assert_eq!(i.env.get("c"), Some(Value::Int(3)));
    }

    #[test]
    fn type_annotations_and_casts_are_scrubbed() {
        let i = run(
            r#"type Flag = boolean
type Shape = { width: number, height: number }
type Mode = "on" | "off"
pub local flag: boolean = true
pub local n = 41 :: number
pub local m = (n + 1) :: number
pub local s: Mode = "on""#,
        );
        assert_eq!(i.env.get("flag"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("n"), Some(Value::Int(41)));
        assert_eq!(i.env.get("m"), Some(Value::Int(42)));
        assert_eq!(i.env.get("s"), Some(Value::str("on")));
    }

    #[test]
    fn export_and_generic_types_are_scrubbed() {
        let i = run(
            r#"export type Wrapper = { value: number }
export type Keys = keyof<Wrapper>
type Pair = Map<string, number>
pub local x: Wrapper = 5
pub local y = 10 :: Array<string>
pub local z: Map<string, Pair> = 7"#,
        );
        assert_eq!(i.env.get("x"), Some(Value::Int(5)));
        assert_eq!(i.env.get("y"), Some(Value::Int(10)));
        assert_eq!(i.env.get("z"), Some(Value::Int(7)));
    }

    #[test]
    fn coroutines_yield_resume_and_share_globals() {
        let i = run(
            r#"pub local ticks = 0
local function gen(start)
  ticks = ticks + 1
  coroutine.yield(start)
  ticks = ticks + 1
  coroutine.yield(start + 1)
  return start + 2
end
local co = coroutine.create(gen)
local ok1, a = coroutine.resume(co, 10)
local ok2, b = coroutine.resume(co)
local ok3, c = coroutine.resume(co)
pub local va = a
pub local vb = b
pub local vc = c
pub local good = ok1 and ok2 and ok3
pub local st = coroutine.status(co)"#,
        );
        assert_eq!(i.env.get("va"), Some(Value::Int(10)));
        assert_eq!(i.env.get("vb"), Some(Value::Int(11)));
        assert_eq!(i.env.get("vc"), Some(Value::Int(12)));
        assert_eq!(i.env.get("good"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("st"), Some(Value::str("dead")));
        assert_eq!(i.env.get("ticks"), Some(Value::Int(2)));
    }

    #[test]
    fn switch_expression_matches_and_returns() {
        let i = run(
            r#"local value = "test"
local var = switch(value)
  case "test"
    return true
  end
  case 1
  end
end
pub local result = var

local function classify(n)
  return switch(n)
    case 1
      return "one"
    end
    case 2
      return "two"
    end
    default
      return "other"
    end
  end
end
pub local a = classify(1)
pub local b = classify(2)
pub local c = classify(9)"#,
        );
        assert_eq!(i.env.get("result"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("a"), Some(Value::str("one")));
        assert_eq!(i.env.get("b"), Some(Value::str("two")));
        assert_eq!(i.env.get("c"), Some(Value::str("other")));
    }

    #[test]
    fn raw_functions() {
        let i = run(
            r#"pub local eq = rawequal(1, 1)
pub local neq = rawequal(1, 2)
pub local tl = rawlen({10, 20, 30})
pub local sl = rawlen("hello")
local t = {}
rawset(t, "k", 7)
pub local got = rawget(t, "k")"#,
        );
        assert_eq!(i.env.get("eq"), Some(Value::Bool(true)));
        assert_eq!(i.env.get("neq"), Some(Value::Bool(false)));
        assert_eq!(i.env.get("tl"), Some(Value::Int(3)));
        assert_eq!(i.env.get("sl"), Some(Value::Int(5)));
        assert_eq!(i.env.get("got"), Some(Value::Int(7)));
    }

    #[test]
    fn do_block_scopes_and_cleans_up() {
        let i = run(
            r#"pub local outer = 1
do
  local inner = 2
  outer = inner + 40
end"#,
        );
        assert_eq!(i.env.get("outer"), Some(Value::Int(42)));
        assert_eq!(i.env.get("inner"), None);
    }

    #[test]
    fn typed_function_and_loop_annotations() {
        let i = run(
            r#"local function clamp(x: number, lo: number): number
  if x < lo then return lo end
  return x
end
pub local sum = 0
for i: number = 1, 3 do
  sum += clamp(i, 2)
end"#,
        );

        assert_eq!(i.env.get("sum"), Some(Value::Int(7)));
    }

    #[test]
    fn collectgarbage_runs_at_safe_point() {

        let i = run(
            r#"do
  local t = {}
  t.self = t
end
collectgarbage()
pub local done = true"#,
        );
        assert_eq!(i.env.get("done"), Some(Value::Bool(true)));
    }

    #[test]
    fn host_can_inject_and_call_native() {
        fn triple(_i: &mut Interpreter, args: Vec<Value>) -> std::result::Result<Vec<Value>, String> {
            let n = args.first().and_then(Value::as_int).ok_or("need int")?;
            Ok(vec![Value::int(n * 3)])
        }
        let mut interp = Interpreter::new();
        interp.env.declare("base", Value::int(7), Mutability::Mutable, Visibility::Pub);
        interp.env.declare("triple", Value::native("triple", triple), Mutability::Const, Visibility::Pub);
        interp.run(&parse(tokenize("pub local out = triple(base)").unwrap()).unwrap()).unwrap();
        assert_eq!(interp.env.get("out"), Some(Value::Int(21)));
    }

    #[test]
    fn host_table_with_rust_metatable() {
        fn idx(_i: &mut Interpreter, args: Vec<Value>) -> std::result::Result<Vec<Value>, String> {
            let key = args.get(1).and_then(Value::as_str).unwrap_or("");
            Ok(vec![if key == "magic" { Value::int(99) } else { Value::nil() }])
        }
        let obj = Value::table();
        let meta = Value::table();
        meta.set_field(Value::str("__index"), Value::native("idx", idx)).unwrap();
        obj.set_metatable(meta).unwrap();

        let mut interp = Interpreter::new();
        interp.env.declare("obj", obj, Mutability::Const, Visibility::Pub);
        interp.run(&parse(tokenize("pub local m = obj.magic").unwrap()).unwrap()).unwrap();
        assert_eq!(interp.env.get("m"), Some(Value::Int(99)));
    }

    #[test]
    fn tables_share_but_scalars_copy() {

        let shared = run(
            r#"local a = {1}
local b = a
b[1] = 99
pub local through_a = a[1]"#,
        );
        assert_eq!(shared.env.get("through_a"), Some(Value::Int(99)));

        let copied = run(
            r#"local x = 5
local y = x
y = 100
pub local original = x"#,
        );
        assert_eq!(copied.env.get("original"), Some(Value::Int(5)));
    }
}
