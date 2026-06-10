use luar::Interpreter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let module_src = r#"
        local M = {}
        function M.add(a, b) return a + b end
        function M.scale(t, k) return t * k end
        M.name = "mathmod"
        return M
    "#;

    let bytes = luar::precompile_source(module_src)?;
    let path = std::env::temp_dir().join("mathmod.luarc");
    std::fs::write(&path, &bytes)?;

    let bytes = std::fs::read(&path)?;
    let module = luar::load_precompiled_module(&bytes)?;

    let mut host = Interpreter::new();
    host.set_global("math", module);
    host.run_source(
        r#"
        pub local sum    = math.add(2, 3)
        pub local scaled = math.scale(sum, 10)
        pub local who    = math.name
    "#,
    )?;

    println!("name   = {}", host.get_global("who").unwrap());
    println!("sum    = {}", host.get_global("sum").unwrap());
    println!("scaled = {}", host.get_global("scaled").unwrap());

    std::fs::remove_file(&path).ok();
    Ok(())
}
