
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("compile") => return compile_cmd(&args[1..]),
        Some("run") => return run_cmd(&args[1..]),
        _ => {}
    }

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/script.luar");

    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let diagnostics = luar::ferrite::check(&source);
    if diagnostics.is_empty() {
        println!("Ferrite: no issues found in script.luar\n");
    } else {
        println!("Ferrite found {} issue(s) in script.luar:", diagnostics.len());
        for d in &diagnostics {
            println!("  script.luar:{d}");
        }
        println!();
    }

    let snippet = "local x = 1\nx = 2\nlocal total = x + 7\nprint(total)";
    let snippet_diags = luar::ferrite::check(snippet);
    if snippet_diags.is_empty() {
        println!("Ferrite on the snippet: no issues\n");
    } else {
        println!("Ferrite on the snippet:");
        for d in snippet_diags {
            println!("  {d}");
        }
        println!();
    }

    match luar::eval_source(&source) {
        Ok(_interp) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("luar error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn compile_cmd(args: &[String]) -> ExitCode {
    let Some(input) = args.first() else {
        eprintln!("usage: compile <in.luar> [out.luarc]");
        return ExitCode::FAILURE;
    };
    let output = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| Path::new(input).with_extension("luarc").to_string_lossy().into_owned());
    let source = match fs::read_to_string(input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match luar::precompile_source(&source) {
        Ok(bytes) => match fs::write(&output, &bytes) {
            Ok(_) => {
                println!("compiled {input} -> {output} ({} bytes)", bytes.len());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("failed to write {output}: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("compile error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_cmd(args: &[String]) -> ExitCode {
    let Some(file) = args.first() else {
        eprintln!("usage: run <file.luar|file.luarc>");
        return ExitCode::FAILURE;
    };
    if file.ends_with(".luarc") {
        let bytes = match fs::read(file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("failed to read {file}: {e}");
                return ExitCode::FAILURE;
            }
        };
        match luar::run_precompiled(&bytes) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("luar error: {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        let source = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("failed to read {file}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let mut interp = luar::Interpreter::new();
        if let Some(parent) = std::path::Path::new(file).parent() {
            if !parent.as_os_str().is_empty() {
                interp.set_module_dir(parent);
            }
        }
        match interp.run_source(&source) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("luar error: {e}");
                ExitCode::FAILURE
            }
        }
    }
}
