use luar::Interpreter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("luar_modules_demo");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("mathmod.luar"),
        "local M = {}\nfunction M.add(a, b) return a + b end\nM.name = \"mathmod\"\nreturn M\n",
    )?;

    let path = dir.join("mathmod.luar");
    let source = std::fs::read_to_string(&path)?;
    let module = Interpreter::load_module(&source)?;
    let mut host = Interpreter::new();
    host.set_global("math", module);
    host.run_source("pub local a = math.add(10, 5)\npub local who = math.name")?;
    println!("[load_module]  who = {}, a = {}", host.get_global("who").unwrap(), host.get_global("a").unwrap());

    let mut host2 = Interpreter::new();
    host2.set_module_dir(&dir);
    host2.run_source(
        r#"local math = require("mathmod")
           pub local b = math.add(2, 40)
           pub local name2 = math.name"#,
    )?;
    println!("[require]      name = {}, b = {}", host2.get_global("name2").unwrap(), host2.get_global("b").unwrap());

    host2.free_module("mathmod");
    println!("[after free]   module functions niled, data kept");

    std::fs::remove_dir_all(&dir).ok();
    Ok(())
}
