
use std::io::Read;
use std::process::ExitCode;

use luar::tokenize;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("tokenize") => run_tokenize(args.get(2).map(String::as_str)),
        Some(other) => {
            eprintln!("unknown command: {other}");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::SUCCESS
        }
    }
}

fn run_tokenize(arg: Option<&str>) -> ExitCode {
    let source = match arg {
        Some("-") | None => {
            let mut buf = String::new();
            if std::io::stdin().read_to_string(&mut buf).is_err() {
                eprintln!("failed to read stdin");
                return ExitCode::FAILURE;
            }
            buf
        }
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("failed to read {path}: {e}");
                return ExitCode::FAILURE;
            }
        },
    };

    match tokenize(&source) {
        Ok(tokens) => {
            for t in &tokens {
                println!("{:>4}:{:<3} {:?} {:?}", t.span.line, t.span.col, t.kind, t.text);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!("LUAR — modular language runtime");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  luar tokenize <file.luar>   tokenize a source file");
    eprintln!("  luar tokenize -             tokenize stdin");
}
