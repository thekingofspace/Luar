
use std::cell::RefCell;
use std::rc::Rc;

use super::coroutine;
use super::interp::{EvalError, Interpreter};
use super::value::Value;
use crate::Error;

pub struct Context {
    interp: Interpreter,
}

impl Default for Context {
    fn default() -> Self {
        Context::new()
    }
}

impl Context {

    pub fn new() -> Self {
        Context { interp: Interpreter::new() }
    }

    pub fn interpreter(&mut self) -> &mut Interpreter {
        &mut self.interp
    }

    pub fn run(&mut self, source: &str) -> Result<Vec<Value>, Error> {
        self.interp.run_source(source)
    }

    pub fn spawn(&mut self, source: &str) -> Result<Value, Error> {
        let _fam = super::gil::FamilyScope::enter(&self.interp.family);
        let program = crate::parse_source(source)?;
        let global = self.interp.env.global_scope();

        let func = Value::function("<script>".to_string(), Vec::new(), false, Rc::new(program), global.clone());
        let state = coroutine::create(func, global, self.interp.family.clone());
        Ok(Value::Coroutine(Rc::new(RefCell::new(state))))
    }

    pub fn resume(&mut self, coro: &Value, args: Vec<Value>) -> Result<Vec<Value>, Error> {
        let _fam = super::gil::FamilyScope::enter(&self.interp.family);
        match coro {
            Value::Coroutine(rc) => Ok(coroutine::resume(rc, args)),
            other => Err(Error::Eval(EvalError(format!(
                "Context::resume expected a thread, got {}",
                other.type_name()
            )))),
        }
    }

    pub fn status(&self, coro: &Value) -> &'static str {
        match coro {
            Value::Coroutine(rc) => rc.borrow().status_str(),
            _ => "not a thread",
        }
    }

    pub fn set_global(&mut self, name: &str, value: Value) {
        self.interp.set_global(name, value);
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        self.interp.get_global(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_share_globals_and_yield_cooperatively() {
        let mut ctx = Context::new();
        ctx.run("pub local tick = 0").unwrap();

        let a = ctx
            .spawn("tick += 1\ncoroutine.yield(tick)\ntick += 1\ncoroutine.yield(tick)")
            .unwrap();
        let b = ctx.spawn("coroutine.yield(tick * 10)").unwrap();

        assert_eq!(ctx.resume(&a, vec![]).unwrap(), vec![Value::Bool(true), Value::Int(1)]);

        assert_eq!(ctx.resume(&b, vec![]).unwrap(), vec![Value::Bool(true), Value::Int(10)]);
        assert_eq!(ctx.resume(&a, vec![]).unwrap(), vec![Value::Bool(true), Value::Int(2)]);

        assert_eq!(ctx.get_global("tick"), Some(Value::Int(2)));
        assert_eq!(ctx.status(&a), "suspended");
        assert_eq!(ctx.status(&b), "suspended");

        assert_eq!(ctx.resume(&b, vec![]).unwrap(), vec![Value::Bool(true)]);
        assert_eq!(ctx.status(&b), "dead");
    }
}
