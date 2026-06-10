
use std::cell::{Cell, RefCell as StdRefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use super::env::{nil_dead_functions_in_scope, scope_parent, scope_values, ScopeRef};
use super::value::{Class, Function, Table, Value};

thread_local! {
    static HEAP: StdRefCell<Heap> = StdRefCell::new(Heap::new());

    static CURRENT_SCRIPT: Cell<u64> = const { Cell::new(0) };

    static NEXT_SCRIPT_ID: Cell<u64> = const { Cell::new(1) };

    static SCRIPT_ROOTS: StdRefCell<HashMap<u64, ScopeRef>> = StdRefCell::new(HashMap::new());
}

pub fn current_script() -> u64 {
    CURRENT_SCRIPT.with(|c| c.get())
}

pub fn new_script_id() -> u64 {
    NEXT_SCRIPT_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

pub(crate) struct ScriptScope(u64);

pub(crate) fn enter_script(id: u64) -> ScriptScope {
    ScriptScope(CURRENT_SCRIPT.with(|c| c.replace(id)))
}

impl Drop for ScriptScope {
    fn drop(&mut self) {
        CURRENT_SCRIPT.with(|c| c.set(self.0));
    }
}

pub(crate) fn register_script_root(id: u64, scope: ScopeRef) {
    SCRIPT_ROOTS.with(|r| r.borrow_mut().insert(id, scope));
}

pub(crate) fn script_root(id: u64) -> Option<ScopeRef> {
    SCRIPT_ROOTS.with(|r| r.borrow().get(&id).cloned())
}

pub(crate) fn unregister_script(id: u64) {
    SCRIPT_ROOTS.with(|r| r.borrow_mut().remove(&id));
}

pub fn live_function_count(script: u64) -> usize {
    HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.functions.retain(|w| w.strong_count() > 0);
        h.functions
            .iter()
            .filter_map(|w| w.upgrade())
            .filter(|f| f.script == script)
            .count()
    })
}

pub fn has_live_functions(script: u64) -> bool {
    live_function_count(script) > 0
}

fn collect_scope_chain(start: ScopeRef, scopes: &mut Vec<ScopeRef>) {
    let mut cur = Some(start);
    while let Some(s) = cur {
        if scopes.iter().any(|e| Rc::ptr_eq(e, &s)) {
            break;
        }
        let parent = scope_parent(&s);
        scopes.push(s);
        cur = parent;
    }
}

pub(crate) fn free_script_functions(script: u64, global: &ScopeRef) {
    let funcs: Vec<Rc<Function>> = HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.functions.retain(|w| w.strong_count() > 0);
        h.functions.iter().filter_map(|w| w.upgrade()).collect()
    });
    for f in &funcs {
        if f.script == script {
            f.dead.set(true);
        }
    }

    let mut scopes: Vec<ScopeRef> = Vec::new();
    collect_scope_chain(global.clone(), &mut scopes);
    let roots: Vec<ScopeRef> = SCRIPT_ROOTS.with(|r| r.borrow().values().cloned().collect());
    for r in roots {
        collect_scope_chain(r, &mut scopes);
    }
    for f in &funcs {
        collect_scope_chain(f.captured.clone(), &mut scopes);
    }
    for s in &scopes {
        nil_dead_functions_in_scope(s);
    }

    let tables: Vec<Weak<std::cell::RefCell<Table>>> = HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.tables.retain(|w| w.strong_count() > 0);
        h.tables.clone()
    });
    for w in &tables {
        if let Some(rc) = w.upgrade() {
            let mut t = rc.borrow_mut();
            for v in t.array.iter_mut() {
                if v.is_dead_function() {
                    *v = Value::Nil;
                }
            }
            for v in t.map.values_mut() {
                if v.is_dead_function() {
                    *v = Value::Nil;
                }
            }
        }
    }
}

const DEFAULT_THRESHOLD: usize = 10_000;

struct Heap {
    tables: Vec<Weak<std::cell::RefCell<Table>>>,
    functions: Vec<Weak<Function>>,
    classes: Vec<Weak<Class>>,
    allocs_since_gc: usize,
    threshold: usize,
    pending: bool,
}

impl Heap {
    fn new() -> Self {
        Heap {
            tables: Vec::new(),
            functions: Vec::new(),
            classes: Vec::new(),
            allocs_since_gc: 0,
            threshold: DEFAULT_THRESHOLD,
            pending: false,
        }
    }
}

pub(crate) fn register_table(rc: &Rc<std::cell::RefCell<Table>>) {
    HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.tables.push(Rc::downgrade(rc));
        note_alloc(&mut h);
    });
}

pub(crate) fn register_function(rc: &Rc<Function>) {
    HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.functions.push(Rc::downgrade(rc));
        note_alloc(&mut h);
    });
}

pub(crate) fn register_class(rc: &Rc<Class>) {
    HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.classes.push(Rc::downgrade(rc));
        note_alloc(&mut h);
    });
}

fn note_alloc(h: &mut Heap) {
    h.allocs_since_gc += 1;
    if h.allocs_since_gc >= h.threshold {
        h.pending = true;
    }
}

pub fn request() {
    HEAP.with(|h| h.borrow_mut().pending = true);
}

pub fn should_collect() -> bool {
    HEAP.with(|h| h.borrow().pending)
}

pub fn set_threshold(n: usize) {
    HEAP.with(|h| h.borrow_mut().threshold = n.max(1));
}

pub fn live_objects() -> usize {
    HEAP.with(|h| {
        let h = h.borrow();
        let tables = h.tables.iter().filter(|w| w.strong_count() > 0).count();
        let funcs = h.functions.iter().filter(|w| w.strong_count() > 0).count();
        tables + funcs
    })
}

pub fn collect(roots: &[Value]) {

    let (tables, functions, classes) = HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.tables.retain(|w| w.strong_count() > 0);
        h.functions.retain(|w| w.strong_count() > 0);
        h.classes.retain(|w| w.strong_count() > 0);
        (h.tables.clone(), h.functions.clone(), h.classes.clone())
    });

    for w in &tables {
        if let Some(rc) = w.upgrade() {
            rc.borrow().gc_mark.set(false);
        }
    }
    for w in &functions {
        if let Some(rc) = w.upgrade() {
            rc.gc_mark.set(false);
        }
    }
    for w in &classes {
        if let Some(rc) = w.upgrade() {
            rc.gc_mark.set(false);
        }
    }

    for value in roots {
        mark_value(value);
    }

    for w in &tables {
        if let Some(rc) = w.upgrade() {
            if !rc.borrow().gc_mark.get() {
                let mut t = rc.borrow_mut();
                t.array.clear();
                t.map.clear();
                t.meta = None;
            }
        }
    }

    HEAP.with(|h| {
        let mut h = h.borrow_mut();
        h.tables.retain(|w| w.strong_count() > 0);
        h.functions.retain(|w| w.strong_count() > 0);
        h.classes.retain(|w| w.strong_count() > 0);
        h.allocs_since_gc = 0;
        h.pending = false;
    });
}

fn mark_value(value: &Value) {
    match value {
        Value::Table(rc) => {
            if rc.borrow().gc_mark.replace(true) {
                return;
            }

            let children: Vec<Value> = {
                let t = rc.borrow();
                let mut c: Vec<Value> = t.array.clone();
                c.extend(t.map.values().cloned());
                if let Some(meta) = &t.meta {
                    c.push(Value::Table(meta.clone()));
                }
                c
            };
            for child in &children {
                mark_value(child);
            }
        }
        Value::Function(rc) => {
            if rc.gc_mark.replace(true) {
                return;
            }

            for child in &scope_values(&rc.captured) {
                mark_value(child);
            }
        }
        Value::Class(rc) => {
            if rc.gc_mark.replace(true) {
                return;
            }
            for m in rc.methods.values() {
                mark_value(m);
            }
            for m in rc.operators.values() {
                mark_value(m);
            }
            for m in rc.getters.values() {
                mark_value(m);
            }
            for m in rc.setters.values() {
                mark_value(m);
            }
            if let Some(c) = &rc.constructor {
                mark_value(c);
            }
            mark_value(&Value::Table(rc.statics.clone()));
            mark_value(&Value::Table(rc.instance_meta.clone()));
            if let Some(p) = &rc.parent {
                mark_value(&Value::Class(p.clone()));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn collects_a_table_cycle() {

        let weak_a;
        {
            let a = Value::table();
            let b = Value::table();
            let (ra, rb) = match (&a, &b) {
                (Value::Table(ra), Value::Table(rb)) => (ra.clone(), rb.clone()),
                _ => unreachable!(),
            };
            ra.borrow_mut().set(Value::str("other"), b.clone()).unwrap();
            rb.borrow_mut().set(Value::str("other"), a.clone()).unwrap();
            weak_a = Rc::downgrade(&ra);
        }

        assert!(weak_a.upgrade().is_some());

        collect(&[]);
        assert!(weak_a.upgrade().is_none(), "cyclic table should be collected");
    }

    #[test]
    fn keeps_reachable_tables() {
        let a = Value::table();
        if let Value::Table(ra) = &a {
            ra.borrow_mut().set(Value::str("self"), a.clone()).unwrap();
        }

        collect(std::slice::from_ref(&a));
        if let Value::Table(ra) = &a {
            assert_eq!(ra.borrow().get(&Value::str("self")), a);
        }
    }
}
