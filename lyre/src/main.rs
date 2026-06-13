mod fs;
mod init;

use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();

    match first.as_deref() {
        Some("init") => {
            if let Err(e) = init::run(args.next()) {
                eprintln!("lyre: init failed: {e}");
                std::process::exit(1);
            }
        }
        Some("help") | Some("--help") | Some("-h") => print_usage(),
        other => {
            let entry = other.unwrap_or("main.luar").to_string();
            run_script(&entry);
        }
    }
}

fn print_usage() {
    println!("lyre — a small runtime for the Luar language");
    println!();
    println!("Usage:");
    println!("  lyre [entry]     run a script (default: main.luar)");
    println!("  lyre init [dir]  scaffold a new project (writes lyre.luard, luari.json, main.luar)");
    println!("  lyre help        show this message");
}

fn run_script(arg: &str) {
    let Some(script) = resolve_entry(Path::new(arg)) else {
        eprintln!("lyre: cannot find script '{arg}'");
        std::process::exit(1);
    };
    let dir = script
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut interp = luar::Interpreter::new();
    interp.set_module_dir(&dir);
    interp.set_source_path(script.clone());
    fs::register(&mut interp);

    let result = if script.extension().map(|e| e == "luarb").unwrap_or(false) {
        match std::fs::read(&script) {
            Ok(bytes) => interp.run_precompiled(&bytes),
            Err(e) => {
                eprintln!("lyre: cannot read '{}': {e}", script.display());
                std::process::exit(1);
            }
        }
    } else {
        match std::fs::read_to_string(&script) {
            Ok(source) => interp.run_source(&source),
            Err(e) => {
                eprintln!("lyre: cannot read '{}': {e}", script.display());
                std::process::exit(1);
            }
        }
    };

    match result {
        Ok(_) => luar::run_pending(),
        Err(e) => {
            eprintln!("lyre: {e}");
            std::process::exit(1);
        }
    }
}

fn resolve_entry(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    for ext in ["luar", "luarb"] {
        let candidate = path.with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if path.is_dir() {
        for name in ["main.luar", "main.luarb", "init.luar", "init.luarb"] {
            let candidate = path.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
