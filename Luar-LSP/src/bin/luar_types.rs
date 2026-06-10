use luar_lsp::infer::BindingKind;
use luar_lsp::resolve::TypeEnv;
use luar_lsp::types::Type;
use std::io::Read;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path: Option<String> = None;
    let mut show_declared = false;
    for arg in args.by_ref() {
        match arg.as_str() {
            "--declared" | "-d" => show_declared = true,
            "--help" | "-h" => {
                print_usage();
                return;
            }
            other => path = Some(other.to_string()),
        }
    }
    let Some(path) = path else {
        print_usage();
        std::process::exit(2);
    };

    let source = if path == "-" {
        let mut buf = String::new();
        if std::io::stdin().read_to_string(&mut buf).is_err() {
            eprintln!("error: could not read stdin");
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: could not read {path}: {e}");
                std::process::exit(1);
            }
        }
    };

    let program = match luar_lsp::parse_source_safe(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let ann = luar_lsp::annotations::scan(&source);
    let mut file_env = TypeEnv::from_program(&program);
    file_env.apply_annotations(&ann);
    let opts = luar_lsp::InferOptions {
        annotations: Some(&ann),
        env: Some(&file_env),
        ..luar_lsp::InferOptions::default()
    };
    let analysis = luar_lsp::identify_program_with(&program, &opts);

    println!("== bindings ==");
    for b in &analysis.bindings {
        let line = match b.line {
            Some(l) => format!("{l:>4}"),
            None => "   ?".to_string(),
        };
        let kind = match b.kind {
            BindingKind::Declare { .. } => "",
            BindingKind::BareAssign => " (bare)",
            BindingKind::Assign => " (assign)",
            BindingKind::Class => " (class)",
            BindingKind::Enum => " (enum)",
            BindingKind::Interface => " (interface)",
            BindingKind::Buff => " (buff)",
            BindingKind::LoopVar => " (loop)",
        };
        println!("{line}  {}: {}{kind}", b.name, b.ty);
    }

    if !analysis.module_returns.is_empty() {
        println!();
        println!("== module returns ==");
        for (i, t) in analysis.module_returns.iter().enumerate() {
            println!("{:>4}  {}", i + 1, t);
        }
    }

    if show_declared {
        let env = TypeEnv::from_analysis(&analysis);

        if !analysis.classes.is_empty() {
            println!();
            println!("== classes ==");
            let mut names: Vec<&String> = analysis.classes.keys().collect();
            names.sort();
            for name in names {
                let c = &analysis.classes[name];
                let mut header = format!("class {name}");
                if let Some(p) = &c.parent {
                    header.push_str(&format!(" extends {p}"));
                }
                if !c.mixins.is_empty() {
                    header.push_str(&format!(" mixin {}", c.mixins.join(", ")));
                }
                if !c.interfaces.is_empty() {
                    header.push_str(&format!(" implements {}", c.interfaces.join(", ")));
                }
                println!("{header}");
                for f in &c.fields {
                    let s = if f.is_static { "static " } else { "" };
                    println!("  {s}{}: {}", f.name, f.ty);
                }
                if let Some(ctor) = &c.constructor {
                    println!("  constructor{ctor}");
                }
                for g in &c.getters {
                    println!("  get {}: {}", g.name, g.ty);
                }
                for (s, _) in &c.setters {
                    println!("  set {s}");
                }
                for m in &c.methods {
                    let s = if m.is_static { "static " } else { "" };
                    println!("  {s}function {}{}", m.name, m.sig);
                }
                for (sym, sig) in &c.operators {
                    println!("  operator {sym}{sig}");
                }
            }
        }

        if !analysis.enums.is_empty() {
            println!();
            println!("== enums ==");
            let mut names: Vec<&String> = analysis.enums.keys().collect();
            names.sort();
            for name in names {
                let e = &analysis.enums[name];
                println!("enum {name}");
                for (v, t) in &e.variants {
                    println!("  {v}: {t}");
                }
            }
        }

        if !analysis.aliases.is_empty() {
            println!();
            println!("== type aliases ==");
            for (name, _) in &analysis.aliases {
                match env.aliases.get(name) {
                    Some(alias) => {
                        let value: Type = env.value_type(&alias.ty);
                        println!("type {name} = {}   (value: {value})", alias.ty);
                    }
                    None => println!("type {name}"),
                }
            }
        }
    }
}

fn print_usage() {
    println!("usage: luar-types <file.luar | -> [--declared]");
    println!();
    println!("Resolves the types of every binding in a LUAR source file.");
    println!("  --declared  also print declared classes, enums, and type aliases");
}
