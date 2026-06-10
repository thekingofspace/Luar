use luar::{Interpreter, Value};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static REGISTRY: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
    static CACHE: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

fn custom_require(_interp: &mut Interpreter, args: Vec<Value>) -> Result<Vec<Value>, String> {
    let name = args
        .first()
        .and_then(Value::as_str)
        .ok_or("require: expected a module name string")?
        .to_string();

    if let Some(cached) = CACHE.with(|c| c.borrow().get(&name).cloned()) {
        return Ok(vec![cached]);
    }

    let src = REGISTRY
        .with(|r| r.borrow().get(&name).copied())
        .ok_or_else(|| format!("require: no module registered as '{name}'"))?;

    let mut module_interp = Interpreter::new();
    module_interp.set_global_fn("require", custom_require);
    let returns = module_interp.run_source(src).map_err(|e| e.to_string())?;
    let module = returns.into_iter().next().unwrap_or(Value::Nil);

    CACHE.with(|c| c.borrow_mut().insert(name, module.clone()));
    Ok(vec![module])
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.insert("util".into(), "local M = {}\nfunction M.double(x) return x * 2 end\nreturn M");
        r.insert(
            "math".into(),
            "local util = require(\"util\")\nlocal M = {}\nfunction M.add(a, b) return a + b end\nfunction M.quad(x) return util.double(util.double(x)) end\nM.name = \"math\"\nreturn M",
        );
    });

    let mut host = Interpreter::new();
    host.set_global_fn("require", custom_require);

    host.run_source(
        r#"local math = require("math")
           pub local sum = math.add(2, 3)
           pub local q = math.quad(5)
           pub local who = math.name"#,
    )?;

    println!("who = {}", host.get_global("who").unwrap());
    println!("sum = {}", host.get_global("sum").unwrap());
    println!("quad(5) = {}", host.get_global("q").unwrap());
    Ok(())
}
