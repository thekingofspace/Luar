pub mod annotations;
pub mod builtins;
pub mod completion;
pub mod infer;
pub mod json;
pub mod lsp;
pub mod project;
pub mod resolve;
pub mod type_syntax;
pub mod types;

pub use annotations::AnnotationSet;
pub use infer::{
    Analysis, Binding, BindingKind, ClassInfo, EnumInfo, InferOptions, identify_expr,
    identify_program, identify_program_with,
};
pub use resolve::{Resolved, TypeEnv, from_luar_type};
pub use types::{FunctionType, ParamInfo, TableType, Type};

const LARGE_STACK_SIZE: usize = 256 * 1024 * 1024;

pub fn on_large_stack<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(LARGE_STACK_SIZE)
        .spawn(f)
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "analysis thread panicked".to_string())
}

pub(crate) fn scoped_large_stack<T, F>(f: F) -> T
where
    T: Send,
    F: FnOnce() -> T + Send,
{
    std::thread::scope(|scope| {
        match std::thread::Builder::new()
            .stack_size(LARGE_STACK_SIZE)
            .spawn_scoped(scope, f)
        {
            Ok(handle) => match handle.join() {
                Ok(v) => v,
                Err(payload) => std::panic::resume_unwind(payload),
            },
            Err(_) => panic!("could not spawn analysis thread"),
        }
    })
}

pub fn parse_source_safe(src: &str) -> Result<Vec<luar::ast::Stmt>, String> {
    scoped_large_stack(|| luar::parse_source(src).map_err(|e| e.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub message: String,
    pub severity: u8,
}

pub fn parse_source_repaired(src: &str) -> (Vec<luar::ast::Stmt>, String) {
    let (program, text, _) = parse_source_repaired_with_errors(src);
    (program, text)
}

pub fn parse_source_repaired_with_errors(
    src: &str,
) -> (Vec<luar::ast::Stmt>, String, Vec<Diagnostic>) {
    scoped_large_stack(|| {
        let mut errors: Vec<Diagnostic> = Vec::new();
        let report = |e: &luar::Error, errors: &mut Vec<Diagnostic>| {
            let (line, col, message) = match e {
                luar::Error::Parse(p) => (p.line, p.col, p.message.clone()),
                luar::Error::Lex(l) => (l.line, l.col, l.message.clone()),
                other => (0, 0, other.to_string()),
            };
            errors.push(Diagnostic {
                line,
                col,
                message,
                severity: 1,
            });
            line
        };

        let first_err = match luar::parse_source(src) {
            Ok(program) => return (program, src.to_string(), errors),
            Err(e) => report(&e, &mut errors),
        };

        let mut text = src.to_string();
        if let Some(fixed) = patch_dangling_accessors(&text) {
            if let Ok(program) = luar::parse_source(&fixed) {
                return (program, fixed, errors);
            }
        }

        let base_err = first_err;
        let lines: Vec<&str> = text.lines().collect();
        let line_count = lines.len() as u32;
        let mut candidate = base_err.min(line_count);
        let mut attempts = 0;
        while candidate >= 1 && attempts < 8 {
            let content = lines
                .get(candidate as usize - 1)
                .map(|l| l.trim())
                .unwrap_or("");
            if !content.is_empty() {
                attempts += 1;
                let attempt = blank_line(&text, candidate);
                if let Ok(program) = luar::parse_source(&attempt) {
                    return (program, attempt, errors);
                }
            }
            candidate -= 1;
        }

        let mut blanked = std::collections::HashSet::new();
        for _ in 0..16 {
            match luar::parse_source(&text) {
                Ok(program) => return (program, text, errors),
                Err(e) => {
                    let line = match &e {
                        luar::Error::Parse(p) => p.line,
                        luar::Error::Lex(l) => l.line,
                        _ => 0,
                    };
                    if line == 0 || !blanked.insert(line) {
                        return (Vec::new(), text, errors);
                    }
                    text = blank_line(&text, line);
                }
            }
        }
        (Vec::new(), text, errors)
    })
}

const BARE_LINE_KEYWORDS: [&str; 12] = [
    "end", "else", "do", "break", "return", "then", "in", "default", "self", "super", "true",
    "false",
];

fn is_bare_name_chain(trimmed: &str) -> bool {
    let t = trimmed.trim_start();
    if t.is_empty() || BARE_LINE_KEYWORDS.contains(&t) {
        return false;
    }
    if !t.contains(['.', ':']) {
        return false;
    }
    let mut chars = t.chars();
    let first = chars.next().unwrap();
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    t.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ':')
        && t.chars()
            .last()
            .map(|c| c.is_alphanumeric() || c == '_')
            .unwrap_or(false)
}

fn patch_dangling_accessors(src: &str) -> Option<String> {
    let mut changed = false;
    let out: Vec<String> = src
        .lines()
        .map(|l| {
            let trimmed = l.trim_end();
            if let Some(prev) = trimmed.strip_suffix(['.', ':']) {
                let prev_ok = !prev.ends_with([':', '.'])
                    && prev
                        .chars()
                        .last()
                        .map(|c| c.is_alphanumeric() || c == '_' || c == ')' || c == ']')
                        .unwrap_or(false);
                if prev_ok {
                    changed = true;
                    return format!("{trimmed}__()");
                }
            }
            if trimmed.ends_with('(') {
                changed = true;
                return format!("{trimmed})");
            }
            if is_bare_name_chain(trimmed) && !trimmed.trim_start().contains("::") {
                changed = true;
                return format!("{trimmed}()");
            }
            l.to_string()
        })
        .collect();
    changed.then(|| out.join("\n"))
}

fn blank_line(src: &str, line: u32) -> String {
    src.lines()
        .enumerate()
        .map(|(i, l)| if i as u32 + 1 == line { "" } else { l })
        .collect::<Vec<&str>>()
        .join("\n")
}

pub fn analyze_source(src: &str) -> Result<Analysis, String> {
    analyze_source_with(src, &InferOptions::default())
}

pub fn analyze_source_with(src: &str, opts: &InferOptions) -> Result<Analysis, String> {
    scoped_large_stack(|| {
        let program = luar::parse_source(src).map_err(|e| e.to_string())?;
        Ok(infer::identify_program_unguarded(&program, opts))
    })
}
