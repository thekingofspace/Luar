
use std::cell::{Cell, RefCell};
use super::fxhash::FxHashMap as HashMap;
use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;

use crate::ast::{Access, Expr, Stmt};

use super::coroutine::CoroState;
use super::env::ScopeRef;
use super::gc;
use super::interp::Interpreter;

#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Table(Rc<RefCell<Table>>),

    Native(Native),

    Function(Rc<Function>),

    Coroutine(Rc<RefCell<CoroState>>),

    Class(Rc<Class>),

    Interface(Rc<Interface>),
}

pub type NativeFn = fn(&mut Interpreter, Vec<Value>) -> Result<Vec<Value>, String>;

#[derive(Clone, Copy)]
pub struct Native {
    pub name: &'static str,
    pub func: NativeFn,
}

pub struct Function {
    pub name: String,
    pub params: Vec<Rc<str>>,

    pub is_vararg: bool,
    pub body: Rc<Vec<Stmt>>,
    pub captured: ScopeRef,

    pub(crate) script: u64,

    pub(crate) dead: Cell<bool>,

    pub(crate) gc_mark: Cell<bool>,

    pub(crate) defined_in: Option<Rc<Class>>,
}

impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("name", &self.name)
            .field("params", &self.params)
            .field("is_vararg", &self.is_vararg)
            .field("script", &self.script)
            .field("defined_in", &self.defined_in.as_ref().map(|c| c.name.clone()))
            .finish()
    }
}

pub struct FieldDef {
    pub name: String,
    pub default: Option<Expr>,
}

pub struct Class {
    pub name: String,
    pub parent: Option<Rc<Class>>,

    pub methods: HashMap<String, Value>,

    pub operators: HashMap<String, Value>,

    pub constructor: Option<Value>,

    pub destructor: Option<Value>,

    pub fields: Vec<FieldDef>,

    pub statics: Rc<RefCell<Table>>,

    pub getters: HashMap<String, Value>,
    pub setters: HashMap<String, Value>,

    pub access: HashMap<String, Access>,

    pub abstracts: HashSet<String>,

    pub finals: HashSet<String>,

    pub is_final: bool,

    pub is_abstract: bool,

    pub interfaces: Vec<Rc<Interface>>,

    pub instance_meta: Rc<RefCell<Table>>,

    pub(crate) gc_mark: Cell<bool>,

    pub(crate) script: u64,
}

pub struct Interface {
    pub name: String,
    pub members: HashSet<String>,
    pub parents: Vec<Rc<Interface>>,
}

impl Interface {

    pub fn is_or_extends(self: &Rc<Self>, other: &Rc<Interface>) -> bool {
        Rc::ptr_eq(self, other) || self.parents.iter().any(|p| p.is_or_extends(other))
    }
}

impl Class {

    pub fn find_method(self: &Rc<Self>, name: &str) -> Option<(Value, Rc<Class>)> {
        let mut cur = self.clone();
        loop {
            if let Some(m) = cur.methods.get(name) {
                return Some((m.clone(), cur));
            }
            cur = cur.parent.clone()?;
        }
    }

    pub fn find_operator(self: &Rc<Self>, mm: &str) -> Option<(Value, Rc<Class>)> {
        let mut cur = self.clone();
        loop {
            if let Some(m) = cur.operators.get(mm) {
                return Some((m.clone(), cur));
            }
            cur = cur.parent.clone()?;
        }
    }

    pub fn find_getter(self: &Rc<Self>, name: &str) -> Option<(Value, Rc<Class>)> {
        let mut cur = self.clone();
        loop {
            if let Some(g) = cur.getters.get(name) {
                return Some((g.clone(), cur));
            }
            cur = cur.parent.clone()?;
        }
    }

    pub fn find_setter(self: &Rc<Self>, name: &str) -> Option<(Value, Rc<Class>)> {
        let mut cur = self.clone();
        loop {
            if let Some(s) = cur.setters.get(name) {
                return Some((s.clone(), cur));
            }
            cur = cur.parent.clone()?;
        }
    }

    pub fn has_member(self: &Rc<Self>, name: &str) -> bool {
        let mut cur = self.clone();
        loop {
            if cur.methods.contains_key(name)
                || cur.getters.contains_key(name)
                || cur.setters.contains_key(name)
                || cur.fields.iter().any(|f| f.name == name)
                || !matches!(cur.statics.borrow().get(&Value::str(name)), Value::Nil)
            {
                return true;
            }
            match cur.parent.clone() {
                Some(p) => cur = p,
                None => return false,
            }
        }
    }

    pub fn has_final_method(self: &Rc<Self>, name: &str) -> bool {
        let mut cur = self.clone();
        loop {
            if cur.finals.contains(name) {
                return true;
            }
            match cur.parent.clone() {
                Some(p) => cur = p,
                None => return false,
            }
        }
    }

    pub fn implements_interface(self: &Rc<Self>, iface: &Rc<Interface>) -> bool {
        let mut cur = self.clone();
        loop {
            if cur.interfaces.iter().any(|i| i.is_or_extends(iface)) {
                return true;
            }
            match cur.parent.clone() {
                Some(p) => cur = p,
                None => return false,
            }
        }
    }

    pub fn member_access(self: &Rc<Self>, name: &str) -> Option<(Access, Rc<Class>)> {
        let mut cur = self.clone();
        loop {
            if let Some(a) = cur.access.get(name) {
                return Some((*a, cur.clone()));
            }
            cur = cur.parent.clone()?;
        }
    }

    pub fn descends_from(self: &Rc<Self>, other: &Rc<Class>) -> bool {
        let mut cur = self.clone();
        loop {
            if Rc::ptr_eq(&cur, other) {
                return true;
            }
            match cur.parent.clone() {
                Some(p) => cur = p,
                None => return false,
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i64),
    Str(Rc<str>),
    Bool(bool),
    Ref(RefKey),
}

#[derive(Clone)]
pub struct RefKey {
    pub(crate) ptr: usize,
    pub value: Value,
}

impl PartialEq for RefKey {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl Eq for RefKey {}

impl std::hash::Hash for RefKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ptr.hash(state);
    }
}

impl Key {
    #[inline]
    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(i) => Value::Int(*i),
            Key::Str(s) => Value::Str(s.clone()),
            Key::Bool(b) => Value::Bool(*b),
            Key::Ref(r) => r.value.clone(),
        }
    }
}

pub(crate) fn value_ref_ptr(v: &Value) -> Option<usize> {
    match v {
        Value::Table(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        Value::Function(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        Value::Class(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        Value::Interface(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        Value::Coroutine(rc) => Some(Rc::as_ptr(rc) as *const () as usize),
        Value::Native(n) => Some(n.func as usize),
        _ => None,
    }
}

pub struct Table {
    pub array: Vec<Value>,
    pub map: HashMap<Key, Value>,

    pub meta: Option<Rc<RefCell<Table>>>,

    pub(crate) gc_mark: Cell<bool>,

    pub(crate) cap: Cell<Option<u64>>,

    pub(crate) is_enum: bool,

    pub(crate) created_by: u64,
}

impl Default for Table {
    fn default() -> Table {
        Table {
            array: Vec::new(),
            map: HashMap::default(),
            meta: None,
            gc_mark: Cell::new(false),
            cap: Cell::new(None),
            is_enum: false,
            created_by: gc::current_script(),
        }
    }
}

impl Table {
    pub fn new() -> Self {
        Table::default()
    }

    pub fn len(&self) -> usize {
        self.array.len()
    }

    pub fn is_empty(&self) -> bool {
        self.array.is_empty() && self.map.is_empty()
    }

    pub fn set_cap(&self, cap: u64) {
        self.cap.set(Some(cap));
    }

    pub fn capacity(&self) -> Option<u64> {
        self.cap.get()
    }

    pub fn entry_count(&self) -> u64 {
        (self.array.len() + self.map.len()) as u64
    }

    pub fn check_room_for_one(&self) -> Result<(), String> {
        if let Some(cap) = self.cap.get() {
            if self.entry_count() >= cap {
                return Err(format!(
                    "buff overflow: this value is capped at a fixed size of {cap} and cannot grow further"
                ));
            }
        }
        Ok(())
    }

    pub fn get(&self, key: &Value) -> Value {
        let Some(k) = value_to_key(key) else {
            return Value::Nil;
        };
        if let Key::Int(i) = k {
            if i >= 1 && (i as usize) <= self.array.len() {
                return self.array[i as usize - 1].clone();
            }
        }
        self.map.get(&k).cloned().unwrap_or(Value::Nil)
    }

    pub fn set(&mut self, key: Value, value: Value) -> Result<(), String> {
        let k = value_to_key(&key)
            .ok_or_else(|| format!("invalid table key of type {}", key.type_name()))?;
        let adds = !matches!(value, Value::Nil)
            && match &k {
                Key::Int(i) if *i >= 1 && (*i as usize) <= self.array.len() => false,
                Key::Int(i) if *i >= 1 && *i as usize == self.array.len() + 1 => true,
                _ => !self.map.contains_key(&k),
            };
        if adds {
            self.check_room_for_one()?;
        }
        if let Key::Int(i) = k {
            if i >= 1 && (i as usize) <= self.array.len() {
                self.array[i as usize - 1] = value;
                return Ok(());
            }
            if i >= 1 && i as usize == self.array.len() + 1 {
                self.array.push(value);
                return Ok(());
            }
        }
        self.map.insert(k, value);
        Ok(())
    }

    pub fn metamethod(&self, name: &str) -> Option<Value> {
        let meta = self.meta.as_ref()?;
        let v = meta.borrow().get(&Value::str(name));
        if matches!(v, Value::Nil) {
            None
        } else {
            Some(v)
        }
    }
}

fn value_to_key(v: &Value) -> Option<Key> {
    match v {
        Value::Int(i) => Some(Key::Int(*i)),
        Value::Bool(b) => Some(Key::Bool(*b)),
        Value::Str(s) => Some(Key::Str(s.clone())),
        Value::Float(f) if f.fract() == 0.0 && f.is_finite() => Some(Key::Int(*f as i64)),
        Value::Nil | Value::Float(_) => None,
        other => value_ref_ptr(other).map(|ptr| {
            Key::Ref(RefKey {
                ptr,
                value: other.clone(),
            })
        }),
    }
}

impl Value {

    #[inline]
    pub fn str(s: impl Into<Rc<str>>) -> Value {
        Value::Str(s.into())
    }

    pub fn nil() -> Value {
        Value::Nil
    }

    pub fn int(i: i64) -> Value {
        Value::Int(i)
    }

    pub fn float(f: f64) -> Value {
        Value::Float(f)
    }

    pub fn boolean(b: bool) -> Value {
        Value::Bool(b)
    }

    pub fn native(name: &'static str, func: NativeFn) -> Value {
        Value::Native(Native { name, func })
    }

    pub fn set_metatable(&self, meta: Value) -> Result<(), String> {
        let Value::Table(t) = self else {
            return Err(format!("set_metatable: expected a table, got {}", self.type_name()));
        };
        match meta {
            Value::Table(m) => t.borrow_mut().meta = Some(m),
            Value::Nil => t.borrow_mut().meta = None,
            other => return Err(format!("metatable must be a table or nil, got {}", other.type_name())),
        }
        Ok(())
    }

    pub fn set_field(&self, key: Value, value: Value) -> Result<(), String> {
        match self {
            Value::Table(t) => t.borrow_mut().set(key, value),
            _ => Err(format!("set_field: expected a table, got {}", self.type_name())),
        }
    }

    pub fn set_native(&self, name: &'static str, func: NativeFn) -> Result<(), String> {
        self.set_field(Value::str(name), Value::native(name, func))
    }

    pub fn field(&self, key: &Value) -> Value {
        match self {
            Value::Table(t) => t.borrow().get(key),
            _ => Value::Nil,
        }
    }

    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn table() -> Value {
        let rc = Rc::new(RefCell::new(Table::new()));
        gc::register_table(&rc);
        Value::Table(rc)
    }

    pub fn function(
        name: String,
        params: Vec<String>,
        is_vararg: bool,
        body: Rc<Vec<Stmt>>,
        captured: ScopeRef,
    ) -> Value {
        Self::function_in(name, params, is_vararg, body, captured, None)
    }

    pub fn function_in(
        name: String,
        params: Vec<String>,
        is_vararg: bool,
        body: Rc<Vec<Stmt>>,
        captured: ScopeRef,
        defined_in: Option<Rc<Class>>,
    ) -> Value {
        let rc = Rc::new(Function {
            name,
            params: params.into_iter().map(Rc::from).collect(),
            is_vararg,
            body,
            captured,
            script: gc::current_script(),
            dead: Cell::new(false),
            gc_mark: Cell::new(false),
            defined_in,
        });
        gc::register_function(&rc);
        Value::Function(rc)
    }

    pub(crate) fn is_dead_function(&self) -> bool {
        matches!(self, Value::Function(f) if f.dead.get())
    }

    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "number",
            Value::Float(_) => "number",
            Value::Str(_) => "string",
            Value::Table(t) => {
                if t.try_borrow().map(|tb| tb.is_enum).unwrap_or(false) {
                    "enum"
                } else {
                    "table"
                }
            }
            Value::Native(_) | Value::Function(_) => "function",
            Value::Coroutine(_) => "thread",
            Value::Class(_) => "class",
            Value::Interface(_) => "interface",
        }
    }
}

#[inline]
pub fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) => (*x as f64) == *y,
        (Value::Float(x), Value::Int(y)) => *x == (*y as f64),
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Table(x), Value::Table(y)) => Rc::ptr_eq(x, y),
        (Value::Native(x), Value::Native(y)) => x.func as usize == y.func as usize,
        (Value::Function(x), Value::Function(y)) => Rc::ptr_eq(x, y),
        (Value::Coroutine(x), Value::Coroutine(y)) => Rc::ptr_eq(x, y),
        (Value::Class(x), Value::Class(y)) => Rc::ptr_eq(x, y),
        (Value::Interface(x), Value::Interface(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Table(t) => {
                let t = t.borrow();
                write!(f, "{{")?;
                let mut first = true;
                for v in &t.array {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                    first = false;
                }
                for (k, v) in &t.map {
                    if !first {
                        write!(f, ", ")?;
                    }
                    match k {
                        Key::Int(i) => write!(f, "[{i}] = {v}")?,
                        Key::Bool(b) => write!(f, "[{b}] = {v}")?,
                        Key::Str(s) => write!(f, "{s} = {v}")?,
                        Key::Ref(r) => write!(f, "[<{}>] = {v}", r.value.type_name())?,
                    }
                    first = false;
                }
                write!(f, "}}")
            }
            Value::Native(n) => write!(f, "function: {}", n.name),
            Value::Function(func) => {
                if func.name.is_empty() {
                    write!(f, "function: anonymous")
                } else {
                    write!(f, "function: {}", func.name)
                }
            }
            Value::Coroutine(_) => write!(f, "thread"),
            Value::Class(c) => write!(f, "class: {}", c.name),
            Value::Interface(i) => write!(f, "interface: {}", i.name),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        values_equal(self, other)
    }
}
