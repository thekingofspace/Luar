
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub use crate::ast::{Mutability, Visibility};

use super::value::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarError {

    Const(String),

    Undefined(String),
}

impl std::fmt::Display for VarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VarError::Const(n) => write!(f, "cannot assign to const '{n}' (only `nil` may free it)"),
            VarError::Undefined(n) => write!(f, "undefined variable '{n}'"),
        }
    }
}

impl std::error::Error for VarError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuffFree {
    Freed,
    NotBuff,
    NotFound,
}

#[derive(Debug, Clone)]
pub struct Variable {
    value: Value,
    mutability: Mutability,
    visibility: Visibility,

    buff_size: Option<u64>,
}

impl Variable {
    pub fn new(value: Value, mutability: Mutability, visibility: Visibility) -> Self {
        Variable { value, mutability, visibility, buff_size: None }
    }

    pub fn new_buff(value: Value, size: u64) -> Self {
        Variable { value, mutability: Mutability::Mutable, visibility: Visibility::Local, buff_size: Some(size) }
    }

    pub fn buff_size(&self) -> Option<u64> {
        self.buff_size
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut Value {
        &mut self.value
    }

    pub fn is_const(&self) -> bool {
        self.mutability == Mutability::Const
    }

    pub fn mutability(&self) -> Mutability {
        self.mutability
    }

    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    pub fn set(&mut self, value: Value) -> Result<(), ()> {
        if self.is_const() && !matches!(value, Value::Nil) {
            return Err(());
        }
        self.value = value;
        Ok(())
    }

    pub fn force_set(&mut self, value: Value) {
        self.value = value;
    }

    pub fn clear(&mut self) {
        self.value = Value::Nil;
    }
}

const SMALL_SCOPE_MAX: usize = 8;

#[derive(Debug)]
pub(crate) enum VarMap {
    Small(Vec<(Rc<str>, Variable)>),
    Big(HashMap<Rc<str>, Variable>),
}

impl Default for VarMap {
    fn default() -> VarMap {
        VarMap::Small(Vec::new())
    }
}

impl VarMap {
    #[inline]
    fn get(&self, name: &str) -> Option<&Variable> {
        match self {
            VarMap::Small(v) => v.iter().find(|(k, _)| &**k == name).map(|(_, var)| var),
            VarMap::Big(m) => m.get(name),
        }
    }

    #[inline]
    fn get_mut(&mut self, name: &str) -> Option<&mut Variable> {
        match self {
            VarMap::Small(v) => v
                .iter_mut()
                .find(|(k, _)| &**k == name)
                .map(|(_, var)| var),
            VarMap::Big(m) => m.get_mut(name),
        }
    }

    #[inline]
    fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    fn insert(&mut self, name: Rc<str>, var: Variable) {
        match self {
            VarMap::Small(v) => {
                if let Some(slot) = v.iter_mut().find(|(k, _)| **k == *name) {
                    slot.1 = var;
                    return;
                }
                if v.len() >= SMALL_SCOPE_MAX {
                    let mut m: HashMap<Rc<str>, Variable> = v.drain(..).collect();
                    m.insert(name, var);
                    *self = VarMap::Big(m);
                } else {
                    v.push((name, var));
                }
            }
            VarMap::Big(m) => {
                m.insert(name, var);
            }
        }
    }

    fn remove(&mut self, name: &str) -> Option<Variable> {
        match self {
            VarMap::Small(v) => {
                let idx = v.iter().position(|(k, _)| &**k == name)?;
                Some(v.swap_remove(idx).1)
            }
            VarMap::Big(m) => m.remove(name),
        }
    }

    fn clear(&mut self) {
        match self {
            VarMap::Small(v) => v.clear(),
            VarMap::Big(m) => m.clear(),
        }
    }

    fn each(&self, mut f: impl FnMut(&Variable)) {
        match self {
            VarMap::Small(v) => {
                for (_, var) in v {
                    f(var);
                }
            }
            VarMap::Big(m) => {
                for var in m.values() {
                    f(var);
                }
            }
        }
    }

    fn each_mut(&mut self, mut f: impl FnMut(&mut Variable)) {
        match self {
            VarMap::Small(v) => {
                for (_, var) in v {
                    f(var);
                }
            }
            VarMap::Big(m) => {
                for var in m.values_mut() {
                    f(var);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Scope {
    vars: VarMap,
    parent: Option<ScopeRef>,
}

pub type ScopeRef = Rc<RefCell<Scope>>;

#[derive(Debug)]
pub struct Environment {
    global: ScopeRef,
    current: ScopeRef,

    module_root: ScopeRef,

    buffs: HashMap<String, Variable>,

    pool: Vec<ScopeRef>,
}

impl Default for Environment {
    fn default() -> Self {
        Environment::new()
    }
}

impl Environment {

    pub fn new() -> Self {
        let global: ScopeRef = Rc::new(RefCell::new(Scope::default()));
        Environment { current: global.clone(), module_root: global.clone(), global, buffs: HashMap::new(), pool: Vec::new() }
    }

    pub fn with_global(global: ScopeRef) -> Self {
        Environment { current: global.clone(), module_root: global.clone(), global, buffs: HashMap::new(), pool: Vec::new() }
    }

    pub fn mark_module_root(&mut self) {
        self.module_root = self.current.clone();
    }

    pub fn module_root_scope(&self) -> ScopeRef {
        self.module_root.clone()
    }

    pub fn declare_module_global(&mut self, name: impl Into<Rc<str>>, value: Value, mutability: Mutability) {
        let var = Variable::new(value, mutability, Visibility::Local);
        self.module_root.borrow_mut().vars.insert(name.into(), var);
    }

    pub fn global_scope(&self) -> ScopeRef {
        self.global.clone()
    }

    pub fn depth(&self) -> usize {
        let mut n = 1;
        let mut scope = self.current.clone();
        loop {
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => {
                    n += 1;
                    scope = p;
                }
                None => break,
            }
        }
        n
    }

    pub fn current_scope_sole_tables(&self) -> Vec<Value> {
        let mut out = Vec::new();
        self.current.borrow().vars.each(|var| {
            if let Value::Table(rc) = var.value() {
                if Rc::strong_count(rc) == 1 {
                    out.push(var.value().clone());
                }
            }
        });
        out
    }

    pub fn clear_current(&mut self) {
        self.current.borrow_mut().vars.clear();
    }

    pub fn scope_force_set(scope: &ScopeRef, name: &str, value: Value) {
        if let Some(var) = scope.borrow_mut().vars.get_mut(name) {
            var.force_set(value);
        }
    }

    pub fn push_scope(&mut self) {
        match self.pool.pop() {
            Some(rc) => {
                {
                    let mut s = rc.borrow_mut();
                    s.vars.clear();
                    s.parent = Some(self.current.clone());
                }
                self.current = rc;
            }
            None => {
                let child = Scope { vars: VarMap::default(), parent: Some(self.current.clone()) };
                self.current = Rc::new(RefCell::new(child));
            }
        }
    }

    pub fn pop_scope(&mut self) {
        let parent = self.current.borrow().parent.clone();
        if let Some(parent) = parent {
            let old = std::mem::replace(&mut self.current, parent);
            if Rc::strong_count(&old) == 1 && self.pool.len() < 64 {
                {
                    let mut s = old.borrow_mut();
                    s.vars.clear();
                    s.parent = None;
                }
                self.pool.push(old);
            }
        }
    }

    pub fn capture(&self) -> ScopeRef {
        self.current.clone()
    }

    pub fn swap_current(&mut self, scope: ScopeRef) -> ScopeRef {
        std::mem::replace(&mut self.current, scope)
    }

    pub fn declare(
        &mut self,
        name: impl Into<Rc<str>>,
        value: Value,
        mutability: Mutability,
        visibility: Visibility,
    ) {
        let var = Variable::new(value, mutability, visibility);
        let target = match visibility {
            Visibility::Pub => &self.global,
            Visibility::Local => &self.current,
        };
        target.borrow_mut().vars.insert(name.into(), var);
    }

    pub fn declare_buff(&mut self, name: impl Into<String>, value: Value, size: u64) {
        self.buffs.insert(name.into(), Variable::new_buff(value, size));
    }

    pub fn buff_size(&self, name: &str) -> Option<u64> {
        self.buffs.get(name).and_then(|v| v.buff_size())
    }

    pub fn free_buff(&mut self, name: &str) -> BuffFree {
        if self.buffs.remove(name).is_some() {
            BuffFree::Freed
        } else if self.contains(name) {
            BuffFree::NotBuff
        } else {
            BuffFree::NotFound
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        let mut scope = self.current.clone();
        loop {
            if scope.borrow().vars.contains_key(name) {
                return true;
            }
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => break,
            }
        }
        self.buffs.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        let mut scope = self.current.clone();
        loop {
            if let Some(var) = scope.borrow().vars.get(name) {
                return Some(var.value().clone());
            }
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => break,
            }
        }
        self.buffs.get(name).map(|v| v.value().clone())
    }

    pub fn assign(&mut self, name: &str, value: Value) -> Result<(), VarError> {
        let mut scope = self.current.clone();
        loop {
            if let Some(var) = scope.borrow_mut().vars.get_mut(name) {
                return var.set(value).map_err(|_| VarError::Const(name.to_string()));
            }
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => break,
            }
        }
        if let Some(var) = self.buffs.get_mut(name) {
            return var.set(value).map_err(|_| VarError::Const(name.to_string()));
        }
        Err(VarError::Undefined(name.to_string()))
    }

    pub fn force_set(&mut self, name: &str, value: Value) -> bool {
        let mut scope = self.current.clone();
        loop {
            if let Some(var) = scope.borrow_mut().vars.get_mut(name) {
                var.force_set(value);
                return true;
            }
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => return false,
            }
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<Variable> {
        let mut scope = self.current.clone();
        loop {
            if let Some(var) = scope.borrow_mut().vars.remove(name) {
                return Some(var);
            }
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => return None,
            }
        }
    }

    pub(crate) fn gc_roots(&self) -> Vec<Value> {
        let mut roots = Vec::new();
        let mut scope = self.current.clone();
        loop {
            scope.borrow().vars.each(|var| {
                roots.push(var.value().clone());
            });
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => break,
            }
        }

        self.module_root.borrow().vars.each(|var| {
            roots.push(var.value().clone());
        });

        for var in self.buffs.values() {
            roots.push(var.value().clone());
        }
        roots
    }
}

pub(crate) fn scope_values(scope: &ScopeRef) -> Vec<Value> {
    let mut out = Vec::new();
    scope.borrow().vars.each(|v| out.push(v.value().clone()));
    out
}

pub(crate) fn scope_parent(scope: &ScopeRef) -> Option<ScopeRef> {
    scope.borrow().parent.clone()
}

pub(crate) fn nil_scope_vars(scope: &ScopeRef) {
    scope.borrow_mut().vars.each_mut(|var| var.force_set(Value::Nil));
}

pub(crate) fn nil_dead_functions_in_scope(scope: &ScopeRef) {
    scope.borrow_mut().vars.each_mut(|var| {
        if var.value().is_dead_function() {
            var.force_set(Value::Nil);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_rejects_assignment_but_nil_frees_it() {
        let mut env = Environment::new();
        env.declare("pi", Value::Int(3), Mutability::Const, Visibility::Local);

        assert!(env.assign("pi", Value::Int(4)).is_err());
        assert_eq!(env.get("pi"), Some(Value::Int(3)));

        assert!(env.assign("pi", Value::Nil).is_ok());
        assert_eq!(env.get("pi"), Some(Value::Nil));
    }

    #[test]
    fn host_can_override_const() {
        let mut env = Environment::new();
        env.declare("pi", Value::Int(3), Mutability::Const, Visibility::Local);
        assert!(env.force_set("pi", Value::Int(4)));
        assert_eq!(env.get("pi"), Some(Value::Int(4)));
    }

    #[test]
    fn locals_are_cleaned_up_on_scope_exit() {
        let mut env = Environment::new();
        env.push_scope();
        env.declare("temp", Value::Int(1), Mutability::Mutable, Visibility::Local);
        assert_eq!(env.get("temp"), Some(Value::Int(1)));
        env.pop_scope();
        assert_eq!(env.get("temp"), None);
    }

    #[test]
    fn pub_escapes_to_global_scope() {
        let mut env = Environment::new();
        env.push_scope();
        env.declare("shared", Value::Bool(true), Mutability::Mutable, Visibility::Pub);
        env.pop_scope();
        assert_eq!(env.get("shared"), Some(Value::Bool(true)));
    }

    #[test]
    fn host_can_force_remove() {
        let mut env = Environment::new();
        env.declare("x", Value::Int(9), Mutability::Const, Visibility::Local);
        assert!(env.remove("x").is_some());
        assert_eq!(env.get("x"), None);
    }

    #[test]
    fn buff_survives_scope_exit_and_is_freed_only_by_freebuff() {
        let mut env = Environment::new();
        env.push_scope();
        env.declare_buff("b", Value::Int(5), 8);
        env.pop_scope();

        assert_eq!(env.get("b"), Some(Value::Int(5)));
        assert_eq!(env.buff_size("b"), Some(8));

        assert_eq!(env.free_buff("b"), BuffFree::Freed);
        assert_eq!(env.get("b"), None);
        assert_eq!(env.free_buff("b"), BuffFree::NotFound);
    }

    #[test]
    fn free_buff_rejects_plain_variables() {
        let mut env = Environment::new();
        env.declare("x", Value::Int(1), Mutability::Mutable, Visibility::Local);
        assert_eq!(env.free_buff("x"), BuffFree::NotBuff);
    }
}
