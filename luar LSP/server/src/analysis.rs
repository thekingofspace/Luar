
use std::collections::{HashMap, HashSet};

use luar::ast::{ClassMember, Expr, Stmt};

#[derive(Clone)]
pub struct Member {
    pub name: String,
    pub is_method: bool,
    pub detail: String,
}

#[derive(Clone)]
pub struct ClassData {
    pub name: String,
    pub parent: Option<String>,
    pub mixins: Vec<String>,
    pub members: Vec<Member>,
}

#[derive(Clone)]
pub struct VarInfo {

    pub ty: String,

    pub mutable: bool,

    pub global: bool,

    pub literals: Vec<String>,
}

#[derive(Default, Clone)]
pub struct FileAnalysis {
    pub classes: Vec<ClassData>,
    pub interfaces: Vec<String>,
    pub aliases: Vec<String>,

    pub alias_targets: HashMap<String, String>,

    pub vars: HashMap<String, VarInfo>,

    pub functions: HashMap<String, String>,

    pub class_ranges: Vec<(String, u32, u32)>,

    pub enums: HashMap<String, EnumInfo>,

    pub module_vars: HashMap<String, String>,

    pub docs: HashMap<String, String>,

    pub signatures: HashMap<String, FnSig>,
}

#[derive(Clone, Default)]
pub struct FnSig {
    pub params: Vec<String>,
    pub ret: String,
}

#[derive(Clone, Default)]
pub struct EnumInfo {
    pub variants: Vec<String>,
    pub global: bool,
}

const MODS: &[&str] = &[
    "pub", "local", "const", "public", "private", "protected", "static", "abstract", "final",
    "override", "export",
];
const PRIMS: &[&str] = &[
    "boolean", "number", "string", "table", "thread", "nil", "any", "unknown", "never", "void",
    "function", "true", "false",
];

pub fn is_primitive(name: &str) -> bool {
    PRIMS.contains(&name)
}

pub fn analyze_file(text: &str) -> Option<FileAnalysis> {
    let stmts = luar::parse_source(text).ok()?;
    let mut fa = FileAnalysis::default();
    for s in &stmts {
        collect(s, &mut fa);
    }
    scan_generic_results(text, &mut fa.vars);
    scan_annotations(text, &mut fa.vars);
    scan_param_types(text, &mut fa.vars);
    scan_functions(text, &mut fa.functions);
    fa.class_ranges = brace_class_ranges(text);
    fa.docs = scan_docs(text);
    fa.signatures = scan_signatures(text);
    inject_prelude_classes(&mut fa);
    Some(fa)
}

pub fn scan_docs(text: &str) -> HashMap<String, String> {
    let mut docs = HashMap::new();
    let mut pending: Vec<String> = Vec::new();
    let mut in_block = false;
    let mut block: Vec<String> = Vec::new();
    for raw in text.lines() {
        let t = raw.trim_start();
        if in_block {
            if let Some(idx) = t.find("]]") {
                block.push(t[..idx].to_string());
                pending = std::mem::take(&mut block);
                in_block = false;
            } else {
                block.push(t.to_string());
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("--[[") {
            if let Some(idx) = rest.find("]]") {
                pending = vec![rest[..idx].to_string()];
            } else {
                in_block = true;
                block = vec![rest.to_string()];
            }
            continue;
        }
        if t.starts_with("--") && !t.starts_with("--#") {
            let c = t.trim_start_matches('-');
            pending.push(c.strip_prefix(' ').unwrap_or(c).to_string());
            continue;
        }
        if t.is_empty() {
            continue;
        }
        if !pending.is_empty() {
            let rendered = render_doc(&pending);
            if !rendered.is_empty() {
                for name in decl_names(t) {
                    docs.entry(name).or_insert_with(|| rendered.clone());
                }
            }
        }
        pending.clear();
    }
    docs
}

fn render_doc(lines: &[String]) -> String {
    let joined = lines.join("\n");
    let trimmed = joined.trim();
    trimmed
        .lines()
        .map(|l| {
            let lt = l.trim_start();
            if let Some(after_at) = lt.strip_prefix('@') {
                let tag: String = after_at.chars().take_while(|c| !c.is_whitespace()).collect();
                let rest = &lt[1 + tag.len()..];
                format!("**@{tag}**{rest}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn decl_names(line: &str) -> Vec<String> {
    let s = strip_mods(line);
    if let Some(r) = s.strip_prefix("function") {
        if r.starts_with(|c: char| c.is_whitespace()) {
            let path: String = r
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.' || *c == ':')
                .collect();
            let last = path.split(['.', ':']).last().unwrap_or("").to_string();
            return if last.is_empty() { Vec::new() } else { vec![last] };
        }
    }
    for kw in ["class", "enum", "interface"] {
        if let Some(r) = s.strip_prefix(kw) {
            if r.starts_with(|c: char| c.is_whitespace()) {
                if let Some((n, _)) = read_ident(r.trim_start()) {
                    return vec![n.to_string()];
                }
            }
        }
    }
    if let Some(r) = s.strip_prefix("type ") {
        if let Some((n, _)) = read_ident(r.trim_start()) {
            return vec![n.to_string()];
        }
    }
    if let Some((n, after)) = read_ident(s) {
        let a = after.trim_start();
        if a.starts_with('.') {
            let field: String = a[1..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            return if field.is_empty() { Vec::new() } else { vec![field] };
        }
        if (a.starts_with(':') && !a.starts_with("::")) || a.starts_with('=') || a.starts_with(',') {
            let lhs = s.split('=').next().unwrap_or(s);
            let mut names = Vec::new();
            for part in lhs.split(',') {
                let nm: String = part.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !nm.is_empty() {
                    names.push(nm);
                }
            }
            return names;
        }
        let _ = n;
    }
    Vec::new()
}

fn inject_prelude_classes(fa: &mut FileAnalysis) {
    if fa.classes.iter().any(|c| c.name == "MonoBehaviour") {
        return;
    }
    let hook = |n: &str| Member { name: n.into(), is_method: true, detail: "lifecycle hook".into() };
    fa.classes.push(ClassData {
        name: "MonoBehaviour".into(),
        parent: None,
        mixins: Vec::new(),
        members: vec![hook("Awake"), hook("Start"), hook("Update"), hook("OnDestroy")],
    });
}

fn scan_aliases(text: &str, targets: &mut HashMap<String, String>) {
    for line in text.lines() {
        let mut t = line.trim_start();
        for kw in ["export ", "pub "] {
            t = t.strip_prefix(kw).map(str::trim_start).unwrap_or(t);
        }
        let Some(rest) = t.strip_prefix("type ") else { continue };
        let Some((name, after)) = read_ident(rest.trim_start()) else { continue };
        if let Some(eq) = after.find('=') {
            let target = resolve_return_type(after[eq + 1..].trim());
            if !target.is_empty() {
                targets.insert(name.to_string(), target);
            }
        }
    }
}

pub fn line_scan(text: &str) -> FileAnalysis {
    let mut fa = FileAnalysis::default();
    scan_vars_lines(text, &mut fa.vars);
    scan_generic_results(text, &mut fa.vars);
    scan_annotations(text, &mut fa.vars);
    scan_functions(text, &mut fa.functions);
    scan_aliases(text, &mut fa.alias_targets);
    fa.classes = scan_classes_lines(text);
    fa.enums = scan_enums_lines(text);
    scan_module_vars_lines(text, &mut fa.module_vars);
    fa.class_ranges = brace_class_ranges(text);
    fa.docs = scan_docs(text);
    fa.signatures = scan_signatures(text);
    inject_prelude_classes(&mut fa);
    fa
}

pub fn merge_classes(cached: &mut Vec<ClassData>, scanned: Vec<ClassData>) {
    for sc in scanned {
        match cached.iter_mut().find(|c| c.name == sc.name) {
            Some(c) => {
                c.parent = sc.parent;
                c.mixins = sc.mixins;
                for m in sc.members {
                    if !c.members.iter().any(|x| x.name == m.name) {
                        c.members.push(m);
                    }
                }
            }
            None => cached.push(sc),
        }
    }
}

fn scan_classes_lines(text: &str) -> Vec<ClassData> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let s = strip_mods(lines[i].trim_start());
        let Some(rest) = s.strip_prefix("class") else {
            i += 1;
            continue;
        };
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            i += 1;
            continue;
        }
        let Some((name, after)) = read_ident(rest.trim_start()) else {
            i += 1;
            continue;
        };
        let (parent, mixins) = parse_class_header(after);

        let (mut depth, mut started, mut j) = (0i32, false, i);
        let mut body_lines: Vec<String> = Vec::new();
        'outer: while j < lines.len() {
            let mut seg = String::new();
            for c in lines[j].chars() {
                if c == '{' {
                    depth += 1;
                    if depth == 1 && !started {
                        started = true;
                        continue;
                    }
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        body_lines.push(std::mem::take(&mut seg));
                        break 'outer;
                    }
                }
                if started && depth >= 1 {
                    seg.push(c);
                }
            }
            if started {
                body_lines.push(seg);
            }
            j += 1;
        }

        let mut members = Vec::new();
        for bl in &body_lines {
            for chunk in split_members(bl) {
                if let Some(m) = scan_member_line(&chunk) {
                    if !members.iter().any(|x: &Member| x.name == m.name) {
                        members.push(m);
                    }
                }
            }
        }
        out.push(ClassData { name: name.to_string(), parent, mixins, members });
        i = j + 1;
    }
    out
}

fn split_members(line: &str) -> Vec<String> {
    const STARTS: &[&str] = &[
        "public", "private", "protected", "static", "abstract", "final", "override", "pub",
        "function", "constructor", "operator", "get", "set",
    ];
    let bytes = line.as_bytes();
    let mut starts = Vec::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < line.len() {
        match bytes[i] {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            _ => {}
        }
        if depth <= 0 {
            let left_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            if left_ok {
                for kw in STARTS {
                    if line[i..].starts_with(kw) {
                        let after = i + kw.len();
                        let right_ok = after >= line.len() || !is_ident_byte(bytes[after]);
                        if right_ok {
                            starts.push(i);
                            break;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    if starts.len() <= 1 {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    for k in 0..starts.len() {
        let s = starts[k];
        let e = if k + 1 < starts.len() { starts[k + 1] } else { line.len() };
        out.push(line[s..e].to_string());
    }
    out
}

pub fn mark_ambient(fa: &mut FileAnalysis) {
    for vi in fa.vars.values_mut() {
        vi.global = true;
    }
    for ei in fa.enums.values_mut() {
        ei.global = true;
    }
}

pub fn visible_enums(files: &[&FileAnalysis], current: &FileAnalysis) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut add = |name: &String, info: &EnumInfo| {
        let e: &mut Vec<String> = out.entry(name.clone()).or_default();
        for v in &info.variants {
            if !e.contains(v) {
                e.push(v.clone());
            }
        }
    };
    for (n, info) in &current.enums {
        add(n, info);
    }
    for f in files {
        for (n, info) in &f.enums {
            if info.global {
                add(n, info);
            }
        }
    }
    out
}

pub fn module_table_exports(text: &str) -> Vec<String> {

    let mut ret: Option<String> = None;
    for line in text.lines() {
        if let Some(r) = line.trim_start().strip_prefix("return ") {
            if let Some((id, rest)) = read_ident(r.trim_start()) {
                if rest.trim().is_empty() {
                    ret = Some(id.to_string());
                }
            }
        }
    }
    let Some(ret) = ret else { return Vec::new() };
    let prefix = format!("{ret}.");
    let mut out = Vec::new();
    let mut add = |field: &str| {
        let f = field.to_string();
        if !f.is_empty() && !out.contains(&f) {
            out.push(f);
        }
    };
    for line in text.lines() {
        let t = line.trim_start();

        if let Some(r) = strip_mods(t).strip_prefix("function") {
            if let Some(rest) = r.trim_start().strip_prefix(&prefix) {
                if let Some((field, _)) = read_ident(rest) {
                    add(field);
                }
            }
        }

        if let Some(rest) = t.strip_prefix(&prefix) {
            if let Some((field, after)) = read_ident(rest) {
                if after.trim_start().starts_with('=') {
                    add(field);
                }
            }
        }
    }
    out
}

fn scan_module_vars_lines(text: &str, out: &mut HashMap<String, String>) {
    for line in text.lines() {
        let t = strip_mods(line.trim_start());
        let Some((name, after)) = read_ident(t) else { continue };
        let Some(rhs) = after.trim_start().strip_prefix('=') else { continue };
        let Some(rest) = rhs.trim_start().strip_prefix("require") else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('(') else { continue };
        let rest = rest.trim_start();
        let Some(q) = rest.chars().next() else { continue };
        if q == '"' || q == '\'' {
            if let Some(end) = rest[q.len_utf8()..].find(q) {
                out.insert(name.to_string(), rest[q.len_utf8()..q.len_utf8() + end].to_string());
            }
        }
    }
}

fn scan_enums_lines(text: &str) -> HashMap<String, EnumInfo> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: HashMap<String, EnumInfo> = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let global = trimmed.starts_with("pub ") || trimmed.starts_with("export ");
        let s = strip_mods(trimmed);
        let Some(rest) = s.strip_prefix("enum") else {
            i += 1;
            continue;
        };
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            i += 1;
            continue;
        }
        let Some((name, _)) = read_ident(rest.trim_start()) else {
            i += 1;
            continue;
        };

        let (mut depth, mut seen, mut j) = (0i32, false, i);
        let mut body = String::new();
        while j < lines.len() {
            for c in lines[j].chars() {
                match c {
                    '{' => {
                        depth += 1;
                        seen = true;
                    }
                    '}' => depth -= 1,
                    _ if seen && depth >= 1 => body.push(c),
                    _ => {}
                }
            }
            body.push('\n');
            if seen && depth <= 0 {
                break;
            }
            j += 1;
        }
        let entry = out.entry(name.to_string()).or_default();
        entry.global |= global;
        for raw in body.split([',', '\n', ';']) {
            let name_part = raw.split('=').next().unwrap_or(raw);
            for w in name_part.split_whitespace() {
                let id: String = w.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !id.is_empty()
                    && id.starts_with(|c: char| c.is_alphabetic() || c == '_')
                    && !entry.variants.contains(&id)
                {
                    entry.variants.push(id);
                }
            }
        }
        i = j + 1;
    }
    out
}

fn parse_class_header(after: &str) -> (Option<String>, Vec<String>) {
    let head = after.split('{').next().unwrap_or(after);
    let parent = find_word(head, "extends")
        .and_then(|p| read_ident(head[p + "extends".len()..].trim_start()))
        .map(|(id, _)| id.to_string());
    let mut mixins = Vec::new();
    if let Some(p) = find_word(head, "mixin") {
        let mut rest = head[p + "mixin".len()..].trim_start();
        while let Some((id, after)) = read_ident(rest) {
            if id == "implements" {
                break;
            }
            mixins.push(id.to_string());
            match after.trim_start().strip_prefix(',') {
                Some(a) => rest = a.trim_start(),
                None => break,
            }
        }
    }
    (parent, mixins)
}

fn scan_member_line(line: &str) -> Option<Member> {
    let t = strip_mods(line.trim_start());
    if let Some(rest) = t.strip_prefix("function") {
        if rest.starts_with(|c: char| c.is_whitespace()) {
            if let Some((name, _)) = read_ident(rest.trim_start()) {
                return Some(Member { name: name.to_string(), is_method: true, detail: "method".into() });
            }
        }
    }
    for kw in ["get", "set"] {
        if let Some(rest) = t.strip_prefix(kw) {
            if rest.starts_with(|c: char| c.is_whitespace()) {
                if let Some((name, _)) = read_ident(rest.trim_start()) {
                    return Some(Member { name: name.to_string(), is_method: false, detail: "property".into() });
                }
            }
        }
    }

    if let Some((name, after)) = read_ident(t) {
        let a = after.trim_start();
        if (a.starts_with(':') && !a.starts_with("::")) || a.starts_with('=') {
            return Some(Member { name: name.to_string(), is_method: false, detail: "field".into() });
        }
    }
    None
}

fn scan_vars_lines(text: &str, vars: &mut HashMap<String, VarInfo>) {
    let param = |vars: &mut HashMap<String, VarInfo>, n: &str| {
        if !n.is_empty() {
            vars.entry(n.to_string())
                .or_insert(VarInfo { ty: String::new(), mutable: true, global: false, literals: Vec::new() });
        }
    };
    for line in text.lines() {
        let t = line.trim_start();

        if let Some(fpos) = find_word(line, "function") {
            let rest = &line[fpos + "function".len()..];
            if let Some(params) = function_params_on_line(line) {
                for (pname, pty) in params {
                    let e = vars.entry(pname).or_insert(VarInfo {
                        ty: String::new(),
                        mutable: true,
                        global: false,
                        literals: Vec::new(),
                    });
                    if e.ty.is_empty() && !pty.is_empty() {
                        e.ty = pty;
                    }
                }

                if let Some((fname, _)) = read_ident(rest.trim_start()) {
                    vars.entry(fname.to_string()).or_insert(VarInfo {
                        ty: "function".into(),
                        mutable: false,
                        global: line.trim_start().starts_with("pub"),
                        literals: Vec::new(),
                    });
                }
            }
        }

        if let Some(r) = t.strip_prefix("for ") {
            let head = r.split(" in ").next().unwrap_or(r).split('=').next().unwrap_or(r);
            for v in head.split(',') {
                param(vars, &v.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>());
            }
        }

        if let Some(after) = t.strip_prefix("buff ") {
            let after = after.trim_start();
            let after_size = after.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_').trim_start();
            let name: String = after_size.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() {
                let rhs = after_size.splitn(2, '=').nth(1).unwrap_or("").trim();
                let ty = infer_rhs(rhs);
                vars.entry(name).or_insert(VarInfo { ty, mutable: true, global: false, literals: Vec::new() });
            }
            continue;
        }

        let mut rest = t;
        let mut mutable = false;
        let mut global = false;
        let mut is_decl = false;
        if let Some(r) = rest.strip_prefix("pub ") {
            global = true;
            is_decl = true;
            rest = r.trim_start();
        }
        if let Some(r) = rest.strip_prefix("local ") {
            mutable = true;
            is_decl = true;
            rest = r.trim_start();
        } else if let Some(r) = rest.strip_prefix("const ") {
            is_decl = true;
            rest = r.trim_start();
        }
        if !is_decl
            || rest.starts_with("function")
            || rest.starts_with("class")
            || rest.starts_with("interface")
            || rest.starts_with("enum")
            || rest.starts_with("type ")
        {
            continue;
        }
        let lhs = rest.split('=').next().unwrap_or(rest);
        let rhs = rest.splitn(2, '=').nth(1).unwrap_or("").trim();
        for (idx, part) in lhs.split(',').enumerate() {
            let name: String = part.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if name.is_empty() {
                continue;
            }
            let mut ty: String = if let Some(colon) = part.find(':') {
                part[colon + 1..].split('=').next().unwrap_or("").trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect()
            } else if idx == 0 {
                infer_rhs(rhs)
            } else {
                String::new()
            };
            if ty.is_empty() && idx == 0 {
                let src: String = rhs.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !src.is_empty() && src == rhs.trim() {
                    match vars.get(&src) {
                        Some(vi) => ty = vi.ty.clone(),
                        None => ty = src,
                    }
                }
            }
            vars.entry(name).or_insert(VarInfo { ty, mutable, global, literals: Vec::new() });
        }
    }
}

fn infer_rhs(rhs: &str) -> String {
    let r = rhs.trim();
    if r.starts_with('"') || r.starts_with('\'') || r.starts_with('`') {
        "string".into()
    } else if r.starts_with("true") || r.starts_with("false") {
        "boolean".into()
    } else if r.starts_with('-') || r.starts_with(|c: char| c.is_ascii_digit()) {
        "number".into()
    } else if r.starts_with('{') {
        "table".into()
    } else if r.starts_with("function") {
        "function".into()
    } else {

        let name: String = r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let rest = r[name.len()..].trim_start();
        if rest.starts_with('(') {
            return if name == "require" { "module".into() } else { name };
        }
        if let Some(after) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('.')) {
            let after = after.trim_start();
            let method: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !method.is_empty() && after[method.len()..].trim_start().starts_with('(') {
                return method;
            }
        }
        String::new()
    }
}

fn parse_params_with_types(s: &str) -> Vec<(String, String)> {
    let bytes = s.as_bytes();
    let mut parts: Vec<&str> = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
            b',' if depth <= 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);

    let mut out = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() || p == "..." {
            continue;
        }
        let name: String = p.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if name.is_empty() {
            continue;
        }
        let after = p[name.len()..].trim_start();
        let ty = after
            .strip_prefix(':')
            .map(|t| t.split('=').next().unwrap_or("").trim().to_string())
            .unwrap_or_default();
        out.push((name, ty));
    }
    out
}

fn function_params_on_line(line: &str) -> Option<Vec<(String, String)>> {
    let fpos = find_word(line, "function")?;
    let rest = &line[fpos + "function".len()..];
    let open = rest.find('(')?;
    let bytes = rest.as_bytes();
    let mut depth = 0i32;
    let mut close = None;
    for i in open..rest.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    Some(parse_params_with_types(&rest[open + 1..close?]))
}

fn scan_param_types(text: &str, vars: &mut HashMap<String, VarInfo>) {
    for line in text.lines() {
        let Some(params) = function_params_on_line(line) else { continue };
        for (name, ty) in params {
            let e = vars.entry(name).or_insert(VarInfo {
                ty: String::new(),
                mutable: true,
                global: false,
                literals: Vec::new(),
            });
            if e.ty.is_empty() && !ty.is_empty() {
                e.ty = ty;
            }
        }
    }
}

fn scan_functions(text: &str, funcs: &mut HashMap<String, String>) {
    for line in text.lines() {
        let Some(fpos) = find_word(line, "function") else { continue };
        let after = line[fpos + "function".len()..].trim_start();
        let Some((name, rest)) = read_ident(after) else { continue };

        let Some(paren) = rest.find(')') else { continue };
        let tail = rest[paren + 1..].trim_start();
        if let Some(rt) = tail.strip_prefix(':') {
            let resolved = resolve_return_type(rt.trim_start());
            if !resolved.is_empty() {
                funcs.insert(name.to_string(), resolved);
            }
        }
    }
}

fn scan_generic_results(text: &str, vars: &mut HashMap<String, VarInfo>) {
    let chars_ident = |c: char| c.is_alphanumeric() || c == '_';
    for line in text.lines() {
        let raw = line.trim_start();
        let t = strip_mods(raw);
        let Some((name, after)) = read_ident(t) else { continue };
        let Some(rhs) = after.trim_start().strip_prefix('=') else { continue };
        let rb: Vec<char> = rhs.chars().collect();
        let mut i = 0;
        while i < rb.len() {
            if rb[i] == '<' && i > 0 && chars_ident(rb[i - 1]) {
                let mut j = i + 1;
                let mut depth = 1;
                while j < rb.len() && depth > 0 {
                    match rb[j] {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    j += 1;
                }
                let after_close: String = rb.get(j + 1..).map(|s| s.iter().collect()).unwrap_or_default();
                if after_close.trim_start().starts_with('(') {
                    let inner: String = rb[i + 1..j].iter().collect();
                    let chosen = inner
                        .split(',')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty() && *s != "_")
                        .next_back()
                        .map(|s| s.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect::<String>());
                    if let Some(ty) = chosen.filter(|s| !s.is_empty()) {
                        let entry = vars.entry(name.to_string()).or_insert_with(|| VarInfo {
                            ty: String::new(),
                            mutable: raw.starts_with("local"),
                            global: raw.starts_with("pub"),
                            literals: Vec::new(),
                        });
                        entry.ty = ty;
                    }
                    break;
                }
                i = j;
            }
            i += 1;
        }
    }
}

pub fn scan_signatures(text: &str) -> HashMap<String, FnSig> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let Some(fpos) = find_word(line, "function") else { continue };
        let after = line[fpos + "function".len()..].trim_start();
        let Some((name, rest)) = read_ident(after) else { continue };
        let last = name.rsplit(['.', ':']).next().unwrap_or(name);
        let Some(open) = rest.find('(') else { continue };
        let Some(close) = matching_paren(rest, open) else { continue };
        let params = split_params(&rest[open + 1..close]);
        let tail = rest[close + 1..].trim_start();
        let ret = tail.strip_prefix(':').map(|r| resolve_return_type(r.trim_start())).unwrap_or_default();
        out.entry(last.to_string()).or_insert(FnSig { params, ret });
    }
    out
}

fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for i in open..s.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_params(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut parts = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth <= 0 => {
                let p = s[start..i].trim();
                if !p.is_empty() {
                    parts.push(p.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        parts.push(last.to_string());
    }
    parts
}

pub fn find_signature<'a>(files: &[&'a FileAnalysis], current: &'a FileAnalysis, name: &str) -> Option<&'a FnSig> {
    current.signatures.get(name).or_else(|| files.iter().find_map(|f| f.signatures.get(name)))
}

pub const TYPE_FUNCS: &[&str] = &[
    "classof", "typeof", "returnof", "valueof", "elementof", "instanceof", "awaited",
    "nonnil", "nonnull", "readonly", "writable", "mutable", "partial", "required",
    "optional", "deep", "shallow", "unwrap", "paramsof", "default", "valuefrom",
];

pub const KEY_TYPE_FUNCS: &[&str] = &["keyof", "nameof", "indexof"];

fn resolve_return_type(s: &str) -> String {
    let s = s.trim();

    if let Some(inner) = s.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
        let trimmed = inner.trim();
        if trimmed.contains(':') {
            return s.to_string();
        }
        if !trimmed.is_empty() && !trimmed.contains(',') && !trimmed.contains('[') {
            let elem = read_type_name(trimmed);
            if !elem.is_empty() {
                return format!("{{{elem}}}");
            }
        }
    }

    if s.contains("->") {
        return "function".into();
    }

    if let Some(inner) = s.strip_prefix('(') {
        let first = inner.split(',').next().unwrap_or("").trim_end_matches(')').trim();
        if first.is_empty() {
            return String::new();
        }
        return resolve_return_type(first);
    }

    if let Some(lt) = s.find('<') {
        let name: String = s[..lt].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() {
            if KEY_TYPE_FUNCS.contains(&name.as_str()) {
                return "string".into();
            }
            if TYPE_FUNCS.contains(&name.as_str()) {
                let inner = s[lt + 1..]
                    .trim()
                    .trim_end_matches('>')
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .trim();
                return resolve_return_type(inner);
            }
        }
    }

    if s.contains('|') || s.contains('&') {
        return s.to_string();
    }

    read_type_name(s)
}

fn find_word(line: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = line[from..].find(word) {
        let i = from + rel;
        let left = i == 0 || !is_ident_byte(line.as_bytes()[i - 1]);
        let after = i + word.len();
        let right = after >= line.len() || !is_ident_byte(line.as_bytes()[after]);
        if left && right {
            return Some(i);
        }
        from = i + word.len();
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn collect(s: &Stmt, fa: &mut FileAnalysis) {
    match s {
        Stmt::Class { name, parent, mixins, members, .. } => {
            let mut ms = Vec::new();
            for m in members {
                match m {
                    ClassMember::Field { name, .. } => {
                        ms.push(Member { name: name.clone(), is_method: false, detail: "field".into() });
                    }
                    ClassMember::Method { name, func, .. } => {
                        ms.push(Member { name: name.clone(), is_method: true, detail: format!("({})", func.params.join(", ")) });
                    }
                    ClassMember::Getter { name, .. } | ClassMember::Setter { name, .. } => {
                        if !ms.iter().any(|x| &x.name == name) {
                            ms.push(Member { name: name.clone(), is_method: false, detail: "property".into() });
                        }
                    }
                    ClassMember::Constructor { .. } | ClassMember::Operator { .. } => {}
                }

                if let Some(func) = member_body(m) {
                    add_params(fa, &func.params);
                    func.body.iter().for_each(|x| collect(x, fa));
                }
            }
            fa.classes.push(ClassData { name: name.clone(), parent: parent.clone(), mixins: mixins.clone(), members: ms });
        }
        Stmt::Interface { name, .. } => fa.interfaces.push(name.clone()),
        Stmt::TypeAlias { name, ty } => {
            fa.aliases.push(name.clone());
            fa.alias_targets.insert(name.clone(), type_to_string(ty));
        }
        Stmt::Enum { visibility, name, variants, .. } => {

            let global = matches!(visibility, luar::ast::Visibility::Pub);
            let entry = fa.enums.entry(name.clone()).or_default();
            entry.global |= global;
            for (v, _) in variants {
                if !entry.variants.contains(v) {
                    entry.variants.push(v.clone());
                }
            }
        }
        Stmt::Declare { names, inits, visibility, mutability, .. } => {
            let mutable = matches!(mutability, luar::ast::Mutability::Mutable);
            let global = matches!(visibility, luar::ast::Visibility::Pub);
            for (i, n) in names.iter().enumerate() {
                let init = inits.get(i);
                let mut ty = init.and_then(infer).unwrap_or_default();
                if ty.is_empty() {
                    if let Some(Expr::Index { base, key }) = init {
                        ty = index_type(base, key, fa);
                    }
                }
                let mut literals = Vec::new();
                if let Some(Expr::Name(src)) = init {
                    if let Some(vi) = fa.vars.get(src) {
                        if ty.is_empty() {
                            ty = vi.ty.clone();
                        }
                        literals = vi.literals.clone();
                    } else if ty.is_empty() {
                        ty = src.clone();
                    }
                    if let Some(path) = fa.module_vars.get(src).cloned() {
                        fa.module_vars.insert(n.clone(), path);
                    }
                }
                if let Some(path) = init.and_then(require_path) {
                    fa.module_vars.insert(n.clone(), path);
                }
                fa.vars.insert(n.clone(), VarInfo { ty, mutable, global, literals });
            }

            inits.iter().for_each(|e| collect_expr(e, fa));
        }

        Stmt::Buff { name, init, .. } => {
            let ty = infer(init).unwrap_or_default();
            fa.vars.insert(name.clone(), VarInfo { ty, mutable: true, global: false, literals: Vec::new() });
            collect_expr(init, fa);
        }

        Stmt::Do(b) => b.iter().for_each(|x| collect(x, fa)),
        Stmt::If { branches, else_block, .. } => {
            for (cond, b) in branches {
                collect_expr(cond, fa);
                b.iter().for_each(|x| collect(x, fa));
            }
            if let Some(b) = else_block {
                b.iter().for_each(|x| collect(x, fa));
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_expr(cond, fa);
            body.iter().for_each(|x| collect(x, fa));
        }
        Stmt::ForNumeric { var, body, .. } => {
            fa.vars.entry(var.clone()).or_insert(VarInfo {
                ty: "number".into(),
                mutable: true,
                global: false,
                literals: Vec::new(),
            });
            body.iter().for_each(|x| collect(x, fa));
        }
        Stmt::ForIn { names, iters, body } => {
            let info = iters.first().and_then(|it| ipairs_pairs_element(it, fa));
            for (idx, n) in names.iter().enumerate() {
                if n == "_" {
                    continue;
                }
                let ty = match &info {
                    Some((is_ipairs, elem)) => {
                        if idx == 0 {
                            if *is_ipairs { "number".to_string() } else { String::new() }
                        } else {
                            elem.clone()
                        }
                    }
                    None => String::new(),
                };
                fa.vars.entry(n.clone()).or_insert(VarInfo {
                    ty,
                    mutable: true,
                    global: false,
                    literals: Vec::new(),
                });
            }
            body.iter().for_each(|x| collect(x, fa));
        }
        Stmt::Return { values, .. } => values.iter().for_each(|e| collect_expr(e, fa)),
        Stmt::Expr(e, _) => collect_expr(e, fa),
        _ => {}
    }
}

fn member_body(m: &ClassMember) -> Option<&luar::ast::FnBody> {
    match m {
        ClassMember::Method { func, .. }
        | ClassMember::Constructor { func }
        | ClassMember::Operator { func, .. }
        | ClassMember::Getter { func, .. }
        | ClassMember::Setter { func, .. } => Some(func),
        ClassMember::Field { .. } => None,
    }
}

fn add_params(fa: &mut FileAnalysis, params: &[String]) {
    for p in params {
        fa.vars
            .entry(p.clone())
            .or_insert(VarInfo { ty: String::new(), mutable: true, global: false, literals: Vec::new() });
    }
}

fn collect_expr(e: &Expr, fa: &mut FileAnalysis) {
    match e {
        Expr::Function { params, body, .. } => {
            add_params(fa, params);
            body.iter().for_each(|x| collect(x, fa));
        }
        Expr::Call { callee, args } => {
            collect_expr(callee, fa);
            args.iter().for_each(|a| collect_expr(a, fa));
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_expr(receiver, fa);
            args.iter().for_each(|a| collect_expr(a, fa));
        }
        Expr::Binary { lhs, rhs, .. } | Expr::Logical { lhs, rhs, .. } => {
            collect_expr(lhs, fa);
            collect_expr(rhs, fa);
        }
        Expr::Unary { expr, .. } => collect_expr(expr, fa),
        Expr::Index { base, key } => {
            collect_expr(base, fa);
            collect_expr(key, fa);
        }
        Expr::Table(entries) => {
            for entry in entries {
                match entry {
                    luar::ast::TableEntry::Positional(v) => collect_expr(v, fa),
                    luar::ast::TableEntry::Keyed { key, value } => {
                        collect_expr(key, fa);
                        collect_expr(value, fa);
                    }
                }
            }
        }
        Expr::Switch { subject, cases, default } => {
            collect_expr(subject, fa);
            for c in cases {
                c.body.iter().for_each(|x| collect(x, fa));
            }
            if let Some(b) = default {
                b.iter().for_each(|x| collect(x, fa));
            }
        }
        _ => {}
    }
}

fn index_type(base: &Expr, key: &Expr, fa: &FileAnalysis) -> String {
    let base_ty = match base {
        Expr::Name(n) => {
            if fa.enums.contains_key(n) {
                return n.clone();
            }
            fa.vars.get(n).map(|v| v.ty.clone()).unwrap_or_default()
        }
        Expr::Index { base, key } => index_type(base, key, fa),
        other => infer(other).unwrap_or_default(),
    };
    if base_ty.is_empty() {
        return String::new();
    }
    let files: [&FileAnalysis; 1] = [fa];
    let resolved = effective_type(&files, &base_ty);
    if let Some(inner) = resolved.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        let inner = inner.trim();
        if inner.contains(':') {
            if let Expr::Str(field) = key {
                for (k, t) in struct_fields(inner) {
                    if &k == field {
                        return t;
                    }
                }
            }
            return String::new();
        }
        return element_of(&resolved);
    }
    String::new()
}

fn struct_fields(inner: &str) -> Vec<(String, String)> {
    let bytes = inner.as_bytes();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut parts: Vec<&str> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' | b'[' | b'(' | b'<' => depth += 1,
            b'}' | b']' | b')' | b'>' => depth -= 1,
            b',' if depth <= 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
        .into_iter()
        .filter_map(|p| {
            let (k, t) = p.split_once(':')?;
            let k = k.trim();
            if k.is_empty() { None } else { Some((k.to_string(), t.trim().to_string())) }
        })
        .collect()
}

fn infer(e: &Expr) -> Option<String> {
    Some(match e {
        Expr::Bool(_) => "boolean".into(),
        Expr::Int(_) | Expr::Float(_) => "number".into(),
        Expr::Str(_) => "string".into(),
        Expr::Nil => "nil".into(),
        Expr::Table(entries) => infer_table(entries),
        Expr::Function { .. } => "function".into(),
        Expr::Call { callee, .. } => match &**callee {
            Expr::Name(n) if n == "require" => "module".into(),
            Expr::Name(n) => n.clone(),
            _ => return None,
        },
        Expr::MethodCall { method, .. } => method.clone(),
        _ => return None,
    })
}

fn infer_table(entries: &[luar::ast::TableEntry]) -> String {
    if entries.is_empty() {
        return "table".into();
    }
    if let Some(t) = array_element(entries) {
        return format!("{{{t}}}");
    }
    let mut fields = Vec::new();
    for e in entries {
        match e {
            luar::ast::TableEntry::Keyed { key, value } => {
                let Some(k) = key_name(key) else { return "table".into() };
                let vt = infer(value).unwrap_or_else(|| "any".into());
                fields.push(format!("{k}: {vt}"));
            }
            luar::ast::TableEntry::Positional(_) => return "table".into(),
        }
    }
    if fields.is_empty() {
        "table".into()
    } else {
        format!("{{ {} }}", fields.join(", "))
    }
}

fn key_name(e: &Expr) -> Option<String> {
    match e {
        Expr::Str(s) => Some(s.clone()),
        Expr::Name(s) => Some(s.clone()),
        _ => None,
    }
}

fn type_to_string(ty: &luar::ast::Type) -> String {
    use luar::ast::Type;
    match ty {
        Type::Named(s) => s.clone(),
        Type::Literal(s) => format!("\"{s}\""),
        Type::Table(fields) => {
            if fields.is_empty() {
                "table".into()
            } else {
                let parts: Vec<String> =
                    fields.iter().map(|(k, t)| format!("{k}: {}", type_to_string(t))).collect();
                format!("{{ {} }}", parts.join(", "))
            }
        }
        Type::Array(inner) => format!("{{{}}}", type_to_string(inner)),
        Type::Optional(inner) => type_to_string(inner),
        Type::Function { .. } => "function".into(),
        Type::Union(parts) => {
            parts.iter().map(type_to_string).collect::<Vec<_>>().join(" | ")
        }
        Type::Intersection(parts) => {
            parts.iter().map(type_to_string).collect::<Vec<_>>().join(" & ")
        }
    }
}

fn ipairs_pairs_element(iter: &Expr, fa: &FileAnalysis) -> Option<(bool, String)> {
    let Expr::Call { callee, args } = iter else { return None };
    let Expr::Name(f) = &**callee else { return None };
    if f != "ipairs" && f != "pairs" {
        return None;
    }
    let coll = args.first()?;
    let coll_ty = match coll {
        Expr::Name(n) => fa.vars.get(n).map(|v| v.ty.clone()).unwrap_or_default(),
        other => infer(other).unwrap_or_default(),
    };
    Some((f == "ipairs", element_of(&coll_ty)))
}

pub fn element_of(ty: &str) -> String {
    if let Some(inner) = ty.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        let inner = inner.trim();
        if inner.starts_with('{') {
            return inner.to_string();
        }
        if inner.contains(':') || inner.contains(',') {
            return String::new();
        }
        return inner.to_string();
    }
    if ty == "table" {
        return "table".to_string();
    }
    String::new()
}

pub fn merge_vars(existing: &mut HashMap<String, VarInfo>, scanned: HashMap<String, VarInfo>) {
    let prev = std::mem::replace(existing, scanned);
    for (name, vi) in existing.iter_mut() {
        let Some(old) = prev.get(name) else { continue };
        let weak = vi.ty.is_empty() || vi.ty == "table";
        let old_rich = !old.ty.is_empty() && old.ty != "table";
        if weak && old_rich {
            vi.ty = old.ty.clone();
        }
        if vi.literals.is_empty() && !old.literals.is_empty() {
            vi.literals = old.literals.clone();
        }
    }
    for (name, vi) in prev {
        existing.entry(name).or_insert(vi);
    }
}

pub fn merge_alias_targets(existing: &mut HashMap<String, String>, scanned: HashMap<String, String>) {
    let prev = std::mem::replace(existing, scanned);
    for (name, target) in prev {
        let old_is_shape = target.starts_with('{');
        match existing.get(&name) {
            Some(cur) if old_is_shape && !cur.starts_with('{') => {
                existing.insert(name, target);
            }
            None => {
                existing.insert(name, target);
            }
            _ => {}
        }
    }
}

fn array_element(entries: &[luar::ast::TableEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut elem: Option<String> = None;
    for e in entries {
        let luar::ast::TableEntry::Positional(v) = e else { return None };
        let t = infer(v)?;
        match &elem {
            None => elem = Some(t),
            Some(prev) if *prev == t => {}
            _ => return None,
        }
    }
    elem
}

fn require_path(e: &Expr) -> Option<String> {
    if let Expr::Call { callee, args } = e {
        if matches!(&**callee, Expr::Name(n) if n == "require") {
            if let Some(Expr::Str(p)) = args.first() {
                return Some(p.clone());
            }
        }
    }
    None
}

fn scan_annotations(text: &str, vars: &mut HashMap<String, VarInfo>) {
    let set = |vars: &mut HashMap<String, VarInfo>, line: &str, name: &str, ty: String, lits: Vec<String>| {
        if let Some(vi) = vars.get_mut(name) {
            if !ty.is_empty() {
                vi.ty = ty;
            }
            if !lits.is_empty() {
                vi.literals = lits;
            }
        } else {
            let t = line.trim_start();
            vars.insert(
                name.to_string(),
                VarInfo { ty, mutable: t.contains("local"), global: t.starts_with("pub"), literals: lits },
            );
        }
    };
    for line in text.lines() {

        let mut annotated = false;
        let stripped = strip_mods(line.trim_start());
        if let Some((name, rest)) = read_ident(stripped) {
            let rest = rest.trim_start();
            if rest.starts_with(':') && !rest.starts_with("::") && !MODS.contains(&name) {
                let expr = rest[1..].split('=').next().unwrap_or("");
                let lits = string_literals(expr);
                let ty = resolve_return_type(expr.trim());
                let ty = if ty.is_empty() && !lits.is_empty() { "string".into() } else { ty };
                if !ty.is_empty() || !lits.is_empty() {
                    set(vars, line, name, ty, lits);
                    annotated = true;
                }
            }
        }

        if !annotated {
            if let Some(idx) = line.find("::") {
                if let Some((name, _)) = read_ident(strip_mods(line.trim_start())) {
                    if !MODS.contains(&name) {
                        let expr = &line[idx + 2..];
                        let lits = string_literals(expr);
                        let ty = read_type_name(expr.trim_start());
                        let ty = if ty.is_empty() && !lits.is_empty() { "string".into() } else { ty };
                        if !ty.is_empty() || !lits.is_empty() {
                            set(vars, line, name, ty, lits);
                        }
                    }
                }
            }
        }
    }
}

fn string_literals(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != c {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

fn brace_class_ranges(text: &str) -> Vec<(String, u32, u32)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let s = strip_mods(lines[i].trim_start());
        if let Some(rest) = s.strip_prefix("class") {
            if rest.starts_with(|c: char| c.is_whitespace()) {
                if let Some((name, _)) = read_ident(rest) {
                    let (mut depth, mut seen, mut j) = (0i32, false, i);
                    while j < lines.len() {
                        for c in lines[j].chars() {
                            if c == '{' {
                                depth += 1;
                                seen = true;
                            } else if c == '}' {
                                depth -= 1;
                            }
                        }
                        if seen && depth <= 0 {
                            break;
                        }
                        j += 1;
                    }
                    out.push((name.to_string(), i as u32, j as u32));
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn strip_mods(mut s: &str) -> &str {
    loop {
        let mut advanced = false;
        for m in MODS {
            if let Some(rest) = s.strip_prefix(m) {
                if rest.starts_with(|c: char| c.is_whitespace()) {
                    s = rest.trim_start();
                    advanced = true;
                    break;
                }
            }
        }
        if !advanced {
            return s;
        }
    }
}

fn read_ident(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if !s.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    let end = s
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

fn read_type_name(s: &str) -> String {
    s.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.').collect()
}

pub fn members_of(files: &[&FileAnalysis], class: &str) -> Vec<Member> {
    let map: HashMap<&str, &ClassData> =
        files.iter().flat_map(|f| f.classes.iter()).map(|c| (c.name.as_str(), c)).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    fn walk<'a>(
        name: &str,
        map: &HashMap<&str, &'a ClassData>,
        out: &mut Vec<Member>,
        seen: &mut HashSet<String>,
        visited: &mut HashSet<String>,
    ) {
        if !visited.insert(name.to_string()) {
            return;
        }
        let Some(cd) = map.get(name) else { return };
        for m in &cd.members {
            if seen.insert(m.name.clone()) {
                out.push(m.clone());
            }
        }
        for mx in &cd.mixins {
            walk(mx, map, out, seen, visited);
        }
        if let Some(p) = &cd.parent {
            walk(p, map, out, seen, visited);
        }
    }
    walk(class, &map, &mut out, &mut seen, &mut HashSet::new());
    out
}

pub fn is_class(files: &[&FileAnalysis], name: &str) -> bool {
    files.iter().any(|f| f.classes.iter().any(|c| c.name == name))
}

pub fn effective_type(files: &[&FileAnalysis], ty: &str) -> String {
    const PASSTHROUGH: &[&str] = &[
        "classof", "instanceof", "nonnil", "nonnull", "optional", "readonly", "writable",
        "mutable", "partial", "required", "deep", "shallow", "unwrap", "default", "awaited",
        "elementof",
    ];

    fn go(files: &[&FileAnalysis], ty: &str, depth: u8) -> String {
        let ty = ty.trim();
        if depth > 6 || ty.is_empty() || is_class(files, ty) {
            return ty.to_string();
        }

        if let Some(open) = ty.find('<') {
            if ty.ends_with('>') {
                let fname = ty[..open].trim();
                if PASSTHROUGH.contains(&fname) {
                    let inner = &ty[open + 1..ty.len() - 1];
                    let first = inner.split(',').next().unwrap_or(inner).trim();
                    let first = first.trim_start_matches('{').trim_end_matches('}').trim();
                    return go(files, first, depth + 1);
                }
            }
        }

        for f in files {
            if let Some(target) = f.alias_targets.get(ty) {
                return go(files, target, depth + 1);
            }
        }
        for f in files {
            if let Some(r) = f.functions.get(ty) {
                return go(files, r, depth + 1);
            }
        }
        for f in files {
            if let Some(vi) = f.vars.get(ty) {
                if vi.global && !vi.ty.is_empty() && vi.ty != ty {
                    return go(files, &vi.ty, depth + 1);
                }
            }
        }
        ty.to_string()
    }
    go(files, ty, 0)
}

pub fn find_var<'a>(files: &[&'a FileAnalysis], current: &'a FileAnalysis, name: &str) -> Option<&'a VarInfo> {
    if let Some(v) = current.vars.get(name) {
        return Some(v);
    }
    files.iter().find_map(|f| f.vars.get(name).filter(|v| v.global))
}

pub fn self_class(current: &FileAnalysis, line: u32) -> Option<&str> {
    current
        .class_ranges
        .iter()
        .find(|(_, s, e)| line >= *s && line <= *e)
        .map(|(n, _, _)| n.as_str())
}

pub fn type_names(files: &[&FileAnalysis]) -> Vec<(String, &'static str)> {
    let mut v = Vec::new();
    for f in files {
        v.extend(f.classes.iter().map(|c| (c.name.clone(), "class")));
        v.extend(f.interfaces.iter().map(|i| (i.clone(), "interface")));
        v.extend(f.aliases.iter().map(|a| (a.clone(), "type alias")));
    }
    v
}
