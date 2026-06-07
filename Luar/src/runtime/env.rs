
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

#[derive(Debug, Clone)]
pub struct Variable {
    value: Value,
    mutability: Mutability,
    visibility: Visibility,
}

impl Variable {
    pub fn new(value: Value, mutability: Mutability, visibility: Visibility) -> Self {
        Variable { value, mutability, visibility }
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

#[derive(Debug, Default)]
pub struct Scope {
    vars: HashMap<String, Variable>,
    parent: Option<ScopeRef>,
}

pub type ScopeRef = Rc<RefCell<Scope>>;

#[derive(Debug)]
pub struct Environment {
    global: ScopeRef,
    current: ScopeRef,

    module_root: ScopeRef,
}

impl Default for Environment {
    fn default() -> Self {
        Environment::new()
    }
}

impl Environment {

    pub fn new() -> Self {
        let global: ScopeRef = Rc::new(RefCell::new(Scope::default()));
        Environment { current: global.clone(), module_root: global.clone(), global }
    }

    pub fn with_global(global: ScopeRef) -> Self {
        Environment { current: global.clone(), module_root: global.clone(), global }
    }

    pub fn mark_module_root(&mut self) {
        self.module_root = self.current.clone();
    }

    pub fn declare_module_global(&mut self, name: impl Into<String>, value: Value, mutability: Mutability) {
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

    pub fn push_scope(&mut self) {
        let child = Scope { vars: HashMap::new(), parent: Some(self.current.clone()) };
        self.current = Rc::new(RefCell::new(child));
    }

    pub fn pop_scope(&mut self) {
        let parent = self.current.borrow().parent.clone();
        if let Some(parent) = parent {
            self.current = parent;
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
        name: impl Into<String>,
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

    pub fn contains(&self, name: &str) -> bool {
        let mut scope = self.current.clone();
        loop {
            if scope.borrow().vars.contains_key(name) {
                return true;
            }
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => return false,
            }
        }
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
                None => return None,
            }
        }
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
                None => return Err(VarError::Undefined(name.to_string())),
            }
        }
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
            for var in scope.borrow().vars.values() {
                roots.push(var.value().clone());
            }
            let parent = scope.borrow().parent.clone();
            match parent {
                Some(p) => scope = p,
                None => break,
            }
        }

        for var in self.module_root.borrow().vars.values() {
            roots.push(var.value().clone());
        }
        roots
    }
}

pub(crate) fn scope_values(scope: &ScopeRef) -> Vec<Value> {
    scope.borrow().vars.values().map(|v| v.value().clone()).collect()
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
}
