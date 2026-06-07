
mod analysis;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use analysis::FileAnalysis;
use luar::ferrite::{self, Severity};
use luar::lexer::{self, TokenKind};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::KEYWORD,
    SemanticTokenType::MODIFIER,
    SemanticTokenType::TYPE,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::CLASS,
];
const T_KEYWORD: u32 = 0;
const T_MODIFIER: u32 = 1;
const T_TYPE: u32 = 2;
const T_FUNCTION: u32 = 3;
const T_VARIABLE: u32 = 4;
const T_STRING: u32 = 5;
const T_NUMBER: u32 = 6;
const T_OPERATOR: u32 = 7;
const T_CLASS: u32 = 8;

const CONTROL: &[&str] = &[
    "if", "then", "elseif", "else", "end", "for", "while", "do", "in", "break", "return", "switch",
    "case", "default", "and", "or", "not", "self", "super",
];

const LITERALS: &[&str] = &["true", "false", "nil"];
const MODIFIERS: &[&str] = &[

    "local", "const", "pub", "public", "private", "protected", "static", "abstract", "final",
    "override", "export", "function", "class", "interface", "type", "extends", "mixin", "implements",
    "enum",
];
const TYPEISH: &[&str] = &["constructor", "operator", "get", "set"];

const CLASS_NAME_AFTER: &[&str] = &["class", "extends", "mixin", "implements", "enum"];
const BUILTINS: &[&str] = &[
    "print", "type", "pcall", "ipairs", "pairs", "next", "setmetatable", "getmetatable", "rawget",
    "rawset", "rawequal", "rawlen", "collectgarbage", "require", "instanceof", "classname",
    "classof", "superclass", "methodsof", "isabstract", "tostring", "tonumber", "run", "spawn",
    "math", "string", "table", "bit32", "os", "coroutine",
];

struct Backend {
    client: Client,

    docs: Mutex<HashMap<Url, String>>,

    cache: Mutex<HashMap<Url, FileAnalysis>>,

    roots: Mutex<Vec<PathBuf>>,

    open: Mutex<std::collections::HashSet<Url>>,
}

impl Backend {

    fn reanalyze(&self, uri: &Url, text: &str) {

        let ambient = uri.path().ends_with(".luard");
        match analysis::analyze_file(text) {
            Some(mut fa) => {
                if ambient {
                    analysis::mark_ambient(&mut fa);
                }
                self.cache.lock().unwrap().insert(uri.clone(), fa);
            }
            None => {
                let mut scan = analysis::line_scan(text);
                if ambient {
                    analysis::mark_ambient(&mut scan);
                }
                let mut cache = self.cache.lock().unwrap();
                match cache.get_mut(uri) {
                    Some(existing) => {
                        existing.vars = scan.vars;
                        existing.functions = scan.functions;
                        existing.alias_targets = scan.alias_targets;
                        existing.class_ranges = scan.class_ranges;
                        existing.enums = scan.enums;
                        existing.docs = scan.docs;

                        analysis::merge_classes(&mut existing.classes, scan.classes);
                    }
                    None => {
                        cache.insert(uri.clone(), scan);
                    }
                }
            }
        }
    }

    fn workspace_luar_paths(&self) -> Vec<PathBuf> {
        let roots = self.roots.lock().unwrap().clone();
        let mut out = Vec::new();
        for root in roots {
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if path.is_dir() {
                        if name != "node_modules" && name != ".git" && !name.starts_with('.') {
                            stack.push(path);
                        }
                    } else if path.extension().map(|e| e == "luar").unwrap_or(false) {
                        out.push(path);
                        if out.len() >= 5000 {
                            return out;
                        }
                    }
                }
            }
        }
        out
    }

    fn prune_deleted(&self) {

        let open = self.open.lock().unwrap().clone();
        let keep = |uri: &Url| open.contains(uri) || uri.to_file_path().map(|p| p.exists()).unwrap_or(true);
        self.cache.lock().unwrap().retain(|uri, _| keep(uri));
        self.docs.lock().unwrap().retain(|uri, _| keep(uri));
    }

    fn load_workspace(&self) -> usize {
        let roots = self.roots.lock().unwrap().clone();
        let mut count = 0;
        for root in roots {
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if path.is_dir() {
                        if name != "node_modules" && name != ".git" && !name.starts_with('.') {
                            stack.push(path);
                        }
                    } else if path.extension().map(|e| e == "luar" || e == "luard").unwrap_or(false) {
                        if let (Ok(text), Ok(uri)) = (std::fs::read_to_string(&path), Url::from_file_path(&path)) {
                            self.reanalyze(&uri, &text);
                            self.docs.lock().unwrap().entry(uri).or_insert(text);
                            count += 1;
                            if count >= 5000 {
                                return count;
                            }
                        }
                    }
                }
            }
        }
        count
    }

    async fn lint(&self, uri: Url, text: &str, version: Option<i32>) {

        if uri.path().ends_with(".luard") {
            self.client.publish_diagnostics(uri, Vec::new(), version).await;
            return;
        }
        let lines: Vec<&str> = text.lines().collect();
        let diagnostics = ferrite::check(text)
            .into_iter()
            .map(|d| {
                let line = d.line.saturating_sub(1);
                let len = lines.get(line as usize).map(|l| l.len()).unwrap_or(0) as u32;
                Diagnostic {
                    range: Range::new(Position::new(line, 0), Position::new(line, len.max(1))),
                    severity: Some(match d.severity {
                        Severity::Error => DiagnosticSeverity::ERROR,
                        Severity::Warning => DiagnosticSeverity::WARNING,
                    }),
                    code: Some(NumberOrString::String(d.code.to_string())),
                    source: Some("ferrite".into()),
                    message: d.message,
                    ..Default::default()
                }
            })
            .collect();
        self.client.publish_diagnostics(uri, diagnostics, version).await;
    }
}

fn call_item(label: &str, kind: CompletionItemKind, detail: String) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail),
        insert_text: Some(format!("{label}($0)")),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }
}

fn completions() -> Vec<CompletionItem> {

    const SNIPPET_WORDS: &[&str] = &["function", "if", "for", "while", "class", "interface", "switch"];

    let mut items = Vec::new();
    for w in CONTROL.iter().chain(MODIFIERS).chain(TYPEISH) {
        if SNIPPET_WORDS.contains(w) {
            continue;
        }
        items.push(CompletionItem {
            label: w.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
    }
    for w in LITERALS {
        items.push(CompletionItem {
            label: w.to_string(),
            kind: Some(CompletionItemKind::CONSTANT),
            ..Default::default()
        });
    }
    for w in BUILTINS {
        items.push(call_item(w, CompletionItemKind::FUNCTION, "builtin".into()));
    }

    let snip = |label: &str, detail: &str, body: &str| CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(detail.to_string()),
        insert_text: Some(body.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    };
    items.push(snip("function", "function … end", "function ${1:name}(${2})\n\t$0\nend"));
    items.push(snip("local function", "local function … end", "local function ${1:name}(${2})\n\t$0\nend"));
    items.push(snip("if", "if … then … end", "if ${1:cond} then\n\t$0\nend"));
    items.push(snip("ifelse", "if … else … end", "if ${1:cond} then\n\t$2\nelse\n\t$0\nend"));
    items.push(snip("for", "numeric for", "for ${1:i} = ${2:1}, ${3:n} do\n\t$0\nend"));
    items.push(snip("forin", "generic for", "for ${1:k}, ${2:v} in ${3:pairs}(${4:t}) do\n\t$0\nend"));
    items.push(snip("while", "while … do … end", "while ${1:cond} do\n\t$0\nend"));
    items.push(snip("class", "class declaration", "class ${1:Name} {\n\t$0\n}"));
    items.push(snip("interface", "interface declaration", "interface ${1:Name} {\n\t$0\n}"));
    items.push(snip("enum", "enum declaration", "enum ${1:Name} {\n\t$0\n}"));
    items.push(snip("switch", "switch expression", "switch(${1:value})\n\tcase ${2:x}\n\t\t$0\n\tend\nend"));
    items
}

const PRIMS: &[&str] = &[
    "boolean", "number", "string", "table", "thread", "nil", "any", "unknown", "never", "void",
    "function", "true", "false",
];

const TYPE_FUNCTIONS: &[(&str, &str)] = &[
    ("classof", "classof<Class> — the instance type of a class"),
    ("typeof", "typeof<value> — the type of a value/binding"),
    ("returnof", "returnof<fn> — a function's return type"),
    ("paramsof", "paramsof<fn> — a function's parameter types"),
    ("keyof", "keyof<T> — the key type of T (string)"),
    ("valueof", "valueof<T> — the value type of T"),
    ("indexof", "indexof<T> — the index/key type of T"),
    ("nameof", "nameof<T> — the name of T as a string"),
    ("elementof", "elementof<{T}> — the element type of an array"),
    ("instanceof", "instanceof<Class> — the instance type"),
    ("awaited", "awaited<T> — unwrap an awaited/optional T"),
    ("nonnil", "nonnil<T> — T with nil removed"),
    ("nonnull", "nonnull<T> — T with nil removed"),
    ("optional", "optional<T> — T | nil"),
    ("readonly", "readonly<T> — an immutable view of T"),
    ("writable", "writable<T> — a mutable view of T"),
    ("mutable", "mutable<T> — a mutable view of T"),
    ("partial", "partial<T> — all fields of T optional"),
    ("required", "required<T> — all fields of T required"),
    ("deep", "deep<T> — a deep variant of T"),
    ("shallow", "shallow<T> — a shallow variant of T"),
    ("unwrap", "unwrap<T> — the underlying type of T"),
    ("default", "default<T> — T with defaults applied"),
    ("valuefrom", "valuefrom<T, key> — the type of T's value at key"),
];

fn type_items(files: &[&FileAnalysis], current: &FileAnalysis) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = PRIMS
        .iter()
        .map(|t| CompletionItem {
            label: t.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some("type".into()),
            ..Default::default()
        })
        .collect();

    for (name, detail) in TYPE_FUNCTIONS {
        let snippet = if *name == "valuefrom" {
            format!("{name}<${{1:T}}, ${{2:key}}>")
        } else {
            format!("{name}<${{1:T}}>")
        };
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some(detail.to_string()),
            insert_text: Some(snippet),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        });
    }
    let mut seen = std::collections::HashSet::new();
    for (name, detail) in analysis::type_names(files) {
        if seen.insert(name.clone()) {
            let kind = match detail {
                "class" => CompletionItemKind::CLASS,
                "interface" => CompletionItemKind::INTERFACE,
                _ => CompletionItemKind::STRUCT,
            };
            items.push(CompletionItem { label: name, kind: Some(kind), detail: Some(detail.into()), ..Default::default() });
        }
    }

    for name in analysis::visible_enums(files, current).keys() {
        if seen.insert(name.clone()) {
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::ENUM),
                detail: Some("enum".into()),
                ..Default::default()
            });
        }
    }
    items
}

fn member_items(files: &[&FileAnalysis], class: &str, op: char) -> Vec<CompletionItem> {
    analysis::members_of(files, class)
        .into_iter()
        .filter(|m| op == '.' || m.is_method)
        .map(|m| {
            let detail = format!("{class} · {}", m.detail);
            if m.is_method {
                call_item(&m.name, CompletionItemKind::METHOD, detail)
            } else {
                CompletionItem { label: m.name, kind: Some(CompletionItemKind::FIELD), detail: Some(detail), ..Default::default() }
            }
        })
        .collect()
}

fn var_items(files: &[&FileAnalysis], current: &FileAnalysis) -> Vec<CompletionItem> {
    let doc_of = |name: &str| {
        current.docs.get(name).map(|d| {
            Documentation::MarkupContent(MarkupContent { kind: MarkupKind::Markdown, value: d.clone() })
        })
    };
    let make = |name: &str, vi: &analysis::VarInfo| {
        let detail = if vi.ty.is_empty() {
            if vi.global { "global".into() } else { "variable".into() }
        } else if vi.global {
            format!("global: {}", vi.ty)
        } else {
            format!("{}: {}", if vi.mutable { "local" } else { "const" }, vi.ty)
        };
        let documentation = doc_of(name);
        if vi.ty == "function" {
            let mut it = call_item(name, CompletionItemKind::FUNCTION, detail);
            it.documentation = documentation;
            return it;
        }
        CompletionItem { label: name.to_string(), kind: Some(CompletionItemKind::VARIABLE), detail: Some(detail), documentation, ..Default::default() }
    };
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for (name, vi) in &current.vars {
        if seen.insert(name.clone()) {
            items.push(make(name, vi));
        }
    }
    for f in files {
        for (name, vi) in &f.vars {
            if vi.global && seen.insert(name.clone()) {
                items.push(make(name, vi));
            }
        }
    }
    items
}

fn member_access(upto: &str) -> Option<(String, char)> {
    let mut chars: Vec<char> = upto.chars().collect();
    let last = *chars.last()?;
    if last != '.' && last != ':' {
        return None;
    }
    chars.pop();
    if last == ':' && chars.last() == Some(&':') {
        return None;
    }
    let name: String = {
        let rev: Vec<char> = chars.iter().rev().take_while(|c| c.is_alphanumeric() || **c == '_').cloned().collect();
        rev.into_iter().rev().collect()
    };
    if name.is_empty() {
        None
    } else {
        Some((name, last))
    }
}

fn is_type_ctx(upto: &str) -> bool {
    let t = upto.trim_end();
    if t.ends_with("::") {
        return true;
    }
    if !t.ends_with(':') || t[..t.len() - 1].contains('=') {
        return false;
    }

    let head = upto.trim_start();
    const DECL: &[&str] = &["local ", "const ", "pub ", "public ", "private ", "protected ", "static ", "type "];
    DECL.iter().any(|k| head.starts_with(k)) || head.contains('(')
}

fn directive_completion(upto: &str) -> Option<Vec<CompletionItem>> {
    let hash = upto.rfind("--#")?;
    let after = upto[hash + 3..].trim_start();

    if after.starts_with("disable") && after.contains(char::is_whitespace) {
        return Some(
            std::iter::once("all")
                .chain(ferrite::CHECKS.iter().copied())
                .map(|c| CompletionItem {
                    label: c.to_string(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    detail: Some("Ferrite check".into()),
                    ..Default::default()
                })
                .collect(),
        );
    }

    let dir = |label: &str, detail: &str| CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::KEYWORD),
        insert_text: Some(format!("{label} ")),
        detail: Some(detail.into()),
        ..Default::default()
    };
    Some(vec![
        dir("disable", "suppress a Ferrite check in this whole file"),
        dir("disable-line", "suppress a Ferrite check on this line"),
        dir("disable-next-line", "suppress a Ferrite check on the next line"),
    ])
}

fn builtin_member_items(recv: &str) -> Option<Vec<CompletionItem>> {
    let members: &[&str] = match recv {
        "math" => &[
            "abs", "ceil", "floor", "round", "sqrt", "sin", "cos", "tan", "asin", "acos", "atan",
            "exp", "log", "pow", "fmod", "modf", "max", "min", "clamp", "sign", "deg", "rad",
            "random", "randomseed", "pi", "huge", "maxinteger", "mininteger",
        ],
        "string" => &[
            "len", "sub", "upper", "lower", "rep", "reverse", "byte", "char", "find", "contains",
            "startswith", "endswith", "trim", "split", "format",
        ],
        "table" => &["insert", "remove", "concat", "unpack", "pack", "sort", "keys"],
        "bit32" => &["band", "bor", "bxor", "bnot", "lshift", "rshift", "arshift"],
        "os" => &["time", "clock"],
        "coroutine" => &["create", "resume", "yield", "status", "close"],
        _ => return None,
    };

    const CONSTS: &[&str] = &["pi", "huge", "maxinteger", "mininteger"];
    Some(
        members
            .iter()
            .map(|m| {
                let detail = format!("{recv}.{m}");
                if CONSTS.contains(m) {
                    CompletionItem { label: m.to_string(), kind: Some(CompletionItemKind::CONSTANT), detail: Some(detail), ..Default::default() }
                } else {
                    call_item(m, CompletionItemKind::FUNCTION, detail)
                }
            })
            .collect(),
    )
}

fn module_member_items(text: &str) -> Vec<CompletionItem> {
    let fa = analysis::analyze_file(text).unwrap_or_else(|| analysis::line_scan(text));
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |label: String, kind: CompletionItemKind, detail: &str| {
        if seen.insert(label.clone()) {
            let documentation = fa.docs.get(&label).map(|d| {
                Documentation::MarkupContent(MarkupContent { kind: MarkupKind::Markdown, value: d.clone() })
            });
            items.push(CompletionItem {
                label,
                kind: Some(kind),
                detail: Some(detail.into()),
                documentation,
                ..Default::default()
            });
        }
    };
    for c in &fa.classes {
        if c.name != "MonoBehaviour" {
            push(c.name.clone(), CompletionItemKind::CLASS, "class");
        }
    }
    for name in fa.enums.keys() {
        push(name.clone(), CompletionItemKind::ENUM, "enum");
    }
    for name in &fa.aliases {
        push(name.clone(), CompletionItemKind::STRUCT, "type");
    }

    for field in analysis::module_table_exports(text) {
        push(field, CompletionItemKind::FIELD, "export");
    }
    items
}

fn luarrc_alias(dir: &std::path::Path, alias: &str) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if let Ok(text) = std::fs::read_to_string(d.join(".luarrc")) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((name, target)) = line.split_once('=') {
                    if name.trim().trim_start_matches('@') == alias {
                        return Some(d.join(target.trim().trim_matches('"')));
                    }
                }
            }
        }
        cur = d.parent();
    }
    None
}

fn resolve_module(from: &Url, path: &str) -> Option<String> {
    let file = from.to_file_path().ok()?;
    let dir = file.parent()?;

    let base = if let Some(rest) = path.strip_prefix('@') {
        if rest == "self" {
            return None;
        }
        let (alias, tail) = rest.split_once('/').map(|(a, t)| (a, Some(t))).unwrap_or((rest, None));
        let target = luarrc_alias(dir, alias)?;
        match tail {
            Some(t) => target.join(t),
            None => target,
        }
    } else {
        dir.join(path.trim_start_matches("./"))
    };

    for candidate in [base.with_extension("luar"), base.with_extension("luard"), base.join("init.luar")] {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            return Some(text);
        }
    }
    None
}

fn enum_member_items(files: &[&FileAnalysis], current: &FileAnalysis, recv: &str) -> Option<Vec<CompletionItem>> {
    let variants = analysis::visible_enums(files, current).remove(recv)?;
    Some(
        variants
            .into_iter()
            .map(|v| CompletionItem {
                label: v,
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(format!("{recv} variant")),
                ..Default::default()
            })
            .collect(),
    )
}

fn require_partial(upto: &str) -> Option<&str> {
    let i = upto.rfind("require")?;
    let rest = upto[i + "require".len()..].trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let inner = &rest[q.len_utf8()..];
    if inner.contains(q) {
        return None;
    }
    Some(inner)
}

fn relative_require(from: &Url, target: &Url) -> Option<String> {
    let from = from.to_file_path().ok()?;
    let target = target.to_file_path().ok()?;
    let from_dir = from.parent()?;
    let f: Vec<_> = from_dir.components().collect();
    let t: Vec<_> = target.components().collect();
    let mut i = 0;
    while i < f.len() && i < t.len() && f[i] == t[i] {
        i += 1;
    }
    let ups = f.len() - i;
    let mut parts: Vec<String> = Vec::new();
    for _ in 0..ups {
        parts.push("..".into());
    }
    for c in &t[i..] {
        parts.push(c.as_os_str().to_string_lossy().replace('\\', "/"));
    }
    let mut rel = parts.join("/");
    if let Some(s) = rel.strip_suffix(".luar") {
        rel = s.to_string();
    }
    if ups == 0 {
        rel = format!("./{rel}");
    }
    Some(rel)
}

fn assignment_target(upto: &str) -> Option<String> {
    let t = upto.trim_end();
    let before = t.strip_suffix('=')?;

    if before.ends_with(['=', '<', '>', '~', '!', ':', '+', '-', '*', '/', '%', '.']) {
        return None;
    }

    let mut words = before.split_whitespace();
    let mut first = words.next()?;
    while matches!(first, "local" | "const" | "pub" | "export" | "public" | "private" | "protected") {
        first = words.next()?;
    }
    let name: String = first.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

fn comparison_lhs(upto: &str) -> Option<String> {
    let t = upto.trim_end();
    let t = t.strip_suffix("==").or_else(|| t.strip_suffix("~="))?.trim_end();
    let name: String = t.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
    (!name.is_empty()).then_some(name)
}

fn literal_items(literals: &[String]) -> Vec<CompletionItem> {
    literals
        .iter()
        .map(|l| CompletionItem {
            label: format!("\"{l}\""),
            kind: Some(CompletionItemKind::VALUE),
            insert_text: Some(format!("\"{l}\"")),
            detail: Some("possible value".into()),
            ..Default::default()
        })
        .collect()
}

fn resolve_receiver(files: &[&FileAnalysis], current: &FileAnalysis, recv: &str, line: u32) -> Option<String> {
    if recv == "self" {
        return analysis::self_class(current, line).map(String::from);
    }
    if let Some(vi) = analysis::find_var(files, current, recv) {

        let ty = analysis::effective_type(files, &vi.ty);
        if analysis::is_class(files, &ty) {
            return Some(ty);
        }
    }
    if analysis::is_class(files, recv) {
        return Some(recv.to_string());
    }
    None
}

fn word_at(line: &str, ch: u32) -> String {
    let chars: Vec<char> = line.chars().collect();
    let i = (ch as usize).min(chars.len());
    let mut start = i;
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    let mut end = i;
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    chars[start..end].iter().collect()
}

fn ih_skip_ws(ch: &[char], i: &mut usize) {
    while *i < ch.len() && (ch[*i] == ' ' || ch[*i] == '\t') {
        *i += 1;
    }
}

fn ih_word(ch: &[char], i: usize, w: &str) -> bool {
    let wl = w.chars().count();
    if i + wl > ch.len() || ch[i..i + wl].iter().collect::<String>() != w {
        return false;
    }
    let after = i + wl;
    after >= ch.len() || !(ch[after].is_alphanumeric() || ch[after] == '_')
}

fn ih_ident(ch: &[char], i: usize) -> (String, usize) {
    if i >= ch.len() || !(ch[i].is_alphabetic() || ch[i] == '_') {
        return (String::new(), i);
    }
    let mut e = i;
    while e < ch.len() && (ch[e].is_alphanumeric() || ch[e] == '_') {
        e += 1;
    }
    (ch[i..e].iter().collect(), e)
}

fn ih_label(mutable: bool, annotated: bool, inferred: Option<String>) -> String {
    let m = if mutable { "mut" } else { "imut" };
    match (annotated, inferred) {
        (true, _) => format!("({m})"),
        (false, Some(t)) => format!(": {m} {t}"),
        (false, None) => format!("({m})"),
    }
}

fn ih_make(line: u32, col: u32, label: String) -> InlayHint {
    InlayHint {
        position: Position::new(line, col),
        label: InlayHintLabel::String(label),
        kind: Some(InlayHintKind::TYPE),
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: Some(false),
        data: None,
    }
}

fn inlay_hints(text: &str, files: &[&FileAnalysis], current: &FileAnalysis) -> Vec<InlayHint> {
    let mut hints = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line_no = lineno as u32;
        let ch: Vec<char> = line.chars().collect();
        let mut i = 0;
        ih_skip_ws(&ch, &mut i);

        let mut is_decl = false;
        let mut mutable = false;
        if ih_word(&ch, i, "pub") {
            i += 3;
            ih_skip_ws(&ch, &mut i);
            is_decl = true;
        }
        if ih_word(&ch, i, "local") {
            i += 5;
            ih_skip_ws(&ch, &mut i);
            mutable = true;
            is_decl = true;
        } else if ih_word(&ch, i, "const") {
            i += 5;
            ih_skip_ws(&ch, &mut i);
            is_decl = true;
        }

        if ih_word(&ch, i, "function") {
            i += 8;
            ih_skip_ws(&ch, &mut i);
            let (name, end) = ih_ident(&ch, i);
            if !name.is_empty() {
                hints.push(ih_make(line_no, end as u32, ih_label(mutable, false, None)));
            }
            continue;
        }

        if ih_word(&ch, i, "class")
            || ih_word(&ch, i, "interface")
            || ih_word(&ch, i, "type")
            || ih_word(&ch, i, "enum")
            || !is_decl
        {
            continue;
        }

        loop {
            ih_skip_ws(&ch, &mut i);
            let (name, name_end) = ih_ident(&ch, i);
            if name.is_empty() {
                break;
            }
            i = name_end;
            ih_skip_ws(&ch, &mut i);

            let mut ann_end: Option<usize> = None;
            if i < ch.len() && ch[i] == ':' && !(i + 1 < ch.len() && ch[i + 1] == ':') {
                i += 1;
                let mut depth = 0i32;
                while i < ch.len() {
                    match ch[i] {
                        '(' | '{' | '[' | '<' => depth += 1,
                        ')' | '}' | ']' | '>' => {
                            if depth > 0 {
                                depth -= 1;
                            }
                        }
                        ',' | '=' if depth == 0 => break,
                        _ => {}
                    }
                    i += 1;
                }
                let mut e = i;
                while e > name_end && (ch[e - 1] == ' ' || ch[e - 1] == '\t') {
                    e -= 1;
                }
                ann_end = Some(e);
            }

            let inferred = if ann_end.is_none() {
                analysis::find_var(files, current, &name).and_then(|vi| {
                    if !vi.literals.is_empty() {
                        Some(vi.literals.iter().map(|l| format!("\"{l}\"")).collect::<Vec<_>>().join(" | "))
                    } else {
                        let et = analysis::effective_type(files, &vi.ty);
                        (!et.is_empty()).then_some(et)
                    }
                })
            } else {
                None
            };

            let col = ann_end.unwrap_or(name_end) as u32;
            hints.push(ih_make(line_no, col, ih_label(mutable, ann_end.is_some(), inferred)));

            ih_skip_ws(&ch, &mut i);
            if i < ch.len() && ch[i] == ',' {
                i += 1;
                continue;
            }
            break;
        }
    }
    hints
}

fn hover_text(files: &[&FileAnalysis], current: &FileAnalysis, word: &str, uri: &Url) -> Option<String> {

    if let Some(path) = current.module_vars.get(word) {
        let ty = if resolve_module(uri, path).is_some() {
            format!("module \"{path}\"")
        } else {
            "*error type*".to_string()
        };
        let kw = match analysis::find_var(files, current, word) {
            Some(vi) if vi.global => "pub",
            Some(vi) if vi.mutable => "local",
            _ => "const",
        };
        return Some(with_doc(format!("```luar\n{kw} {word}: {ty}\n```"), current, word));
    }
    if let Some(vi) = analysis::find_var(files, current, word) {
        let kw = if vi.global { "pub" } else if vi.mutable { "local" } else { "const" };
        let ty = if vi.ty.is_empty() {
            "?".to_string()
        } else if !vi.literals.is_empty() {
            vi.literals.iter().map(|l| format!("\"{l}\"")).collect::<Vec<_>>().join(" | ")
        } else {
            let et = analysis::effective_type(files, &vi.ty);
            if analysis::is_class(files, &et) || analysis::is_primitive(&et) {
                et
            } else {
                "any".to_string()
            }
        };
        let mutability = if vi.mutable { "mutable" } else { "immutable" };
        let scope = if vi.global { ", global" } else { "" };
        return Some(with_doc(format!("```luar\n{kw} {word}: {ty}\n```\n*{mutability}{scope}*"), current, word));
    }
    if analysis::is_class(files, word) {
        return Some(with_doc(format!("```luar\nclass {word}\n```"), current, word));
    }
    if let Some(variants) = analysis::visible_enums(files, current).get(word) {
        return Some(with_doc(format!("```luar\nenum {word} {{ {} }}\n```", variants.join(", ")), current, word));
    }
    if let Some(ret) = current.functions.get(word) {
        let sig = if ret.is_empty() { format!("function {word}(...)") } else { format!("function {word}(...): {ret}") };
        return Some(with_doc(format!("```luar\n{sig}\n```"), current, word));
    }
    if analysis::is_primitive(word) {
        return Some(format!("**{word}** - a built-in type"));
    }
    if let Some(d) = current.docs.get(word) {
        return Some(d.clone());
    }
    None
}

fn with_doc(base: String, current: &FileAnalysis, word: &str) -> String {
    match current.docs.get(word) {
        Some(d) => format!("{base}\n\n---\n\n{d}"),
        None => base,
    }
}

const LIBS: &[&str] = &["coroutine", "math", "string", "table", "bit32", "os"];

fn classify_ident(text: &str) -> u32 {
    if LIBS.contains(&text) {
        T_VARIABLE
    } else if CONTROL.contains(&text) {
        T_KEYWORD
    } else if MODIFIERS.contains(&text) {
        T_MODIFIER
    } else if TYPEISH.contains(&text) {
        T_TYPE
    } else if BUILTINS.contains(&text) {
        T_FUNCTION
    } else {
        T_VARIABLE
    }
}

fn semantic_tokens(text: &str) -> Vec<SemanticToken> {
    let tokens = match lexer::tokenize(text) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    let (mut prev_line, mut prev_start) = (0u32, 0u32);

    let mut expect_class = false;
    let mut class_list = false;
    for tok in &tokens {

        if tok.kind == TokenKind::Delimiter {
            if tok.text == "," && class_list {
                expect_class = true;
            } else {
                expect_class = false;
                class_list = false;
            }
            continue;
        }
        let ttype = match tok.kind {
            TokenKind::Identifier if CLASS_NAME_AFTER.contains(&tok.text.as_str()) => {
                expect_class = true;
                class_list = tok.text == "mixin" || tok.text == "implements";
                classify_ident(&tok.text)
            }
            TokenKind::Identifier if expect_class => {
                expect_class = false;
                T_CLASS
            }
            TokenKind::Identifier => {
                expect_class = false;
                class_list = false;
                classify_ident(&tok.text)
            }
            TokenKind::Str | TokenKind::InterpStr => T_STRING,
            TokenKind::Int | TokenKind::Float => T_NUMBER,
            TokenKind::Operator => T_OPERATOR,
            TokenKind::Comment => continue,
            TokenKind::Eof => continue,
            TokenKind::Delimiter => continue,
        };

        if ttype == T_VARIABLE {
            continue;
        }

        let line = tok.span.line.saturating_sub(1);
        let start = tok.span.col.saturating_sub(1);
        let length = tok.text.chars().count() as u32;
        if length == 0 || tok.text.contains('\n') {
            continue;
        }

        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 { start - prev_start } else { start };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: ttype,
            token_modifiers_bitset: 0,
        });
        prev_line = line;
        prev_start = start;
    }
    out
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {

        let mut roots = Vec::new();
        if let Some(folders) = &params.workspace_folders {
            for f in folders {
                if let Ok(p) = f.uri.to_file_path() {
                    roots.push(p);
                }
            }
        }
        #[allow(deprecated)]
        if roots.is_empty() {
            if let Some(uri) = &params.root_uri {
                if let Ok(p) = uri.to_file_path() {
                    roots.push(p);
                }
            }
        }
        *self.roots.lock().unwrap() = roots;

        Ok(InitializeResult {
            server_info: Some(ServerInfo { name: "luar-lsp-server".into(), version: Some("0.1.0".into()) }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        ".".into(), ":".into(), "#".into(), "/".into(), "\"".into(), "=".into(),
                    ]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
                        legend: SemanticTokensLegend {
                            token_types: TOKEN_TYPES.to_vec(),
                            token_modifiers: vec![],
                        },
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                        range: Some(false),
                        ..Default::default()
                    }),
                ),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {

        let n = self.load_workspace();
        self.client
            .log_message(MessageType::INFO, format!("LUAR language server ready — indexed {n} file(s)"))
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.open.lock().unwrap().insert(doc.uri.clone());
        self.reanalyze(&doc.uri, &doc.text);
        self.docs.lock().unwrap().insert(doc.uri.clone(), doc.text.clone());
        self.lint(doc.uri, &doc.text, Some(doc.version)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {

        if let Some(change) = params.content_changes.into_iter().last() {
            let uri = params.text_document.uri;
            self.reanalyze(&uri, &change.text);
            self.docs.lock().unwrap().insert(uri.clone(), change.text.clone());
            self.lint(uri, &change.text, Some(params.text_document.version)).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.open.lock().unwrap().remove(&params.text_document.uri);
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let p = params.text_document_position;
        let uri = p.text_document.uri;
        let pos = p.position;
        let text = self.docs.lock().unwrap().get(&uri).cloned().unwrap_or_default();
        let line = text.lines().nth(pos.line as usize).unwrap_or("").to_string();
        let upto: String = line.chars().take(pos.character as usize).collect();

        if let Some(items) = directive_completion(&upto) {
            return Ok(Some(CompletionResponse::Array(items)));
        }

        self.prune_deleted();

        if let Some(partial) = require_partial(&upto) {

            let start = pos.character.saturating_sub(partial.chars().count() as u32);
            let range = Range::new(Position::new(pos.line, start), Position::new(pos.line, pos.character));
            let mut items = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let current_path = uri.to_file_path().ok();
            for path in self.workspace_luar_paths() {
                if current_path.as_ref() == Some(&path) {
                    continue;
                }
                let Ok(target) = Url::from_file_path(&path) else { continue };
                if let Some(rel) = relative_require(&uri, &target) {
                    if seen.insert(rel.clone()) {
                        items.push(CompletionItem {
                            label: rel.clone(),
                            kind: Some(CompletionItemKind::MODULE),
                            detail: Some("module".into()),
                            filter_text: Some(rel.clone()),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(range, rel))),
                            ..Default::default()
                        });
                    }
                }
            }
            return Ok(Some(CompletionResponse::Array(items)));
        }

        let cache = self.cache.lock().unwrap();
        let files: Vec<&FileAnalysis> = cache.values().collect();
        let empty = FileAnalysis::default();
        let current = cache.get(&uri).unwrap_or(&empty);

        if let Some((recv, op)) = member_access(&upto) {

            if let Some(class) = resolve_receiver(&files, current, &recv, pos.line) {
                return Ok(Some(CompletionResponse::Array(member_items(&files, &class, op))));
            }

            if let Some(items) = builtin_member_items(&recv) {
                return Ok(Some(CompletionResponse::Array(items)));
            }

            if let Some(items) = enum_member_items(&files, current, &recv) {
                return Ok(Some(CompletionResponse::Array(items)));
            }

            if let Some(path) = current.module_vars.get(&recv) {
                if let Some(src) = resolve_module(&uri, path) {
                    return Ok(Some(CompletionResponse::Array(module_member_items(&src))));
                }
            }

            if op == ':'
                && (is_type_ctx(&upto) || analysis::find_var(&files, current, &recv).is_none())
            {
                return Ok(Some(CompletionResponse::Array(type_items(&files, current))));
            }

            return Ok(Some(CompletionResponse::Array(Vec::new())));
        }

        if let Some(lhs) = comparison_lhs(&upto) {
            if let Some(vi) = analysis::find_var(&files, current, &lhs) {
                if !vi.literals.is_empty() {
                    return Ok(Some(CompletionResponse::Array(literal_items(&vi.literals))));
                }
            }
        }

        if let Some(lhs) = assignment_target(&upto) {
            if let Some(vi) = analysis::find_var(&files, current, &lhs) {
                if !vi.literals.is_empty() {
                    return Ok(Some(CompletionResponse::Array(literal_items(&vi.literals))));
                }
            }
        }

        if is_type_ctx(&upto) {
            return Ok(Some(CompletionResponse::Array(type_items(&files, current))));
        }

        let mut items = completions();
        items.extend(var_items(&files, current));

        for name in analysis::visible_enums(&files, current).keys() {
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::ENUM),
                detail: Some("enum".into()),
                ..Default::default()
            });
        }
        for (name, detail) in analysis::type_names(&files) {
            let kind = match detail {
                "class" => CompletionItemKind::CLASS,
                "interface" => CompletionItemKind::INTERFACE,
                _ => CompletionItemKind::STRUCT,
            };
            items.push(CompletionItem { label: name, kind: Some(kind), detail: Some(detail.into()), ..Default::default() });
        }
        let mut seen = std::collections::HashSet::new();
        items.retain(|i| seen.insert(i.label.clone()));
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let text = self.docs.lock().unwrap().get(&uri).cloned().unwrap_or_default();
        let cache = self.cache.lock().unwrap();
        let files: Vec<&FileAnalysis> = cache.values().collect();
        let empty = FileAnalysis::default();
        let current = cache.get(&uri).unwrap_or(&empty);
        Ok(Some(inlay_hints(&text, &files, current)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let p = params.text_document_position_params;
        let uri = p.text_document.uri;
        let text = self.docs.lock().unwrap().get(&uri).cloned().unwrap_or_default();
        let line = text.lines().nth(p.position.line as usize).unwrap_or("");
        let word = word_at(line, p.position.character);
        if word.is_empty() {
            return Ok(None);
        }
        let cache = self.cache.lock().unwrap();
        let files: Vec<&FileAnalysis> = cache.values().collect();
        let empty = FileAnalysis::default();
        let current = cache.get(&uri).unwrap_or(&empty);
        Ok(hover_text(&files, current, &word, &uri).map(|value| Hover {
            contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }),
            range: None,
        }))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let text = self.docs.lock().unwrap().get(&params.text_document.uri).cloned();
        let data = text.map(|t| semantic_tokens(&t)).unwrap_or_default();
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens { result_id: None, data })))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: Mutex::new(HashMap::new()),
        cache: Mutex::new(HashMap::new()),
        roots: Mutex::new(Vec::new()),
        open: Mutex::new(std::collections::HashSet::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
