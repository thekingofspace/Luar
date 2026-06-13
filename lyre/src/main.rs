use std::path::{Path, PathBuf};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "main.luar".to_string());
    let Some(script) = resolve_entry(Path::new(&arg)) else {
        eprintln!("luar-runtime: cannot find script '{arg}'");
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

    let result = if script.extension().map(|e| e == "luarb").unwrap_or(false) {
        match std::fs::read(&script) {
            Ok(bytes) => interp.run_precompiled(&bytes),
            Err(e) => {
                eprintln!("luar-runtime: cannot read '{}': {e}", script.display());
                std::process::exit(1);
            }
        }
    } else {
        match std::fs::read_to_string(&script) {
            Ok(source) => interp.run_source(&source),
            Err(e) => {
                eprintln!("luar-runtime: cannot read '{}': {e}", script.display());
                std::process::exit(1);
            }
        }
    };

    match result {
        Ok(_) => luar::run_pending(),
        Err(e) => {
            eprintln!("luar-runtime: {e}");
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
