fn wait(_i: &mut luar::Interpreter, args: Vec<luar::Value>) -> Result<Vec<luar::Value>, String> {
    let secs = match args.first() {
        Some(luar::Value::Int(i)) => *i as f64,
        Some(luar::Value::Float(f)) => *f,
        _ => 0.0,
    };
    luar::blocking(move || std::thread::sleep(std::time::Duration::from_secs_f64(secs)))?;
    Ok(vec![])
}

fn main() {
    let scripts = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let entry = scripts.join("main.luar");
    let source = match std::fs::read_to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {e}", entry.display());
            std::process::exit(1);
        }
    };
    let mut interp = luar::Interpreter::new();
    interp.set_module_dir(&scripts);
    interp.set_source_path(entry.clone());
    interp.set_global_fn("wait", wait);
    match interp.run_source(&source) {
        Ok(returned) => {
            if !returned.is_empty() {
                println!("main.luar returned {} value(s)", returned.len());
            }
            luar::run_pending();
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
